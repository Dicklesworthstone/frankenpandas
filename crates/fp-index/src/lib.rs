#![forbid(unsafe_code)]
#![warn(rustdoc::broken_intra_doc_links)]

//! Row-label / index machinery for **frankenpandas** — every
//! `DataFrame` and `Series` in fp-frame carries an [`Index`] that
//! pairs labels with positional row indices, plus the alignment
//! algebra that pandas users expect from `Series + Series` and
//! `DataFrame.align()`.
//!
//! ## Core types
//!
//! - [`Index`]: the canonical row-label container. Internally a
//!   typed `Vec<IndexLabel>` plus an O(1) label-lookup hashmap
//!   built lazily on first use. Pandas `Index` shape: monotonic
//!   probes, duplicate handling, range-style construction
//!   (`Index::from_range(0..N)`), name metadata.
//! - [`IndexLabel`]: typed label enum — `Int64(i64)`, `Float64(f64)`,
//!   `Utf8(String)`, `Bool(bool)`, `Datetime64(i64)`,
//!   `Timedelta64(i64)`. Lets one `Index` carry mixed-type labels
//!   without erasing to strings.
//! - [`MultiIndex`]: hierarchical multi-level index for
//!   pandas-style row MultiIndex DataFrames. Each level is itself
//!   a `Vec<IndexLabel>` plus an integer codes array.
//! - [`MultiIndexOrIndex`]: sum-type for code paths that accept
//!   either flat `Index` or `MultiIndex`.
//! - [`DuplicateKeep`]: enum controlling `keep='first' | 'last'
//!   | False` semantics in `Index.duplicated` /
//!   `Index.drop_duplicates` etc.
//!
//! ## Alignment algebra
//!
//! Binary ops between two pandas-shaped frames need to align rows
//! by label. The aligner builds an [`AlignmentPlan`] (or
//! [`MultiAlignmentPlan`] for N-way joins) that the caller then
//! applies to each side's value buffers:
//!
//! - [`align`] dispatches on [`AlignMode`] (`Left`, `Right`,
//!   `Inner`, `Outer`).
//! - [`align_inner`], [`align_left`], [`align_union`]: direct
//!   single-mode entry points.
//! - [`leapfrog_union`] / [`leapfrog_intersection`]: N-way row
//!   alignment via a leapfrog merge over already-sorted indexes
//!   (used by [`multi_way_align`]).
//! - [`validate_alignment_plan`]: sanity check (lengths match,
//!   indices in bounds).
//!
//! ## Date / time helpers
//!
//! Pandas `pd.date_range` / `pd.timedelta_range` analogs:
//!
//! - [`timedelta_range`]: pandas-style timedelta range builder.
//! - [`apply_date_offset`] / [`apply_date_offset_to_nanos`]:
//!   evaluate a [`DateOffset`] against an anchor timestamp.
//! - [`infer_freq`] / [`infer_freq_from_timestamps`] /
//!   [`infer_freq_from_nanos`]: pandas-style frequency inference
//!   from a sample of timestamps.
//! - [`format_datetime_ns`]: render a nanosecond-since-epoch i64
//!   as the canonical `YYYY-MM-DD HH:MM:SS[.f]` string used in
//!   IndexLabel display and IO formatters.
//!
//! ## Error reporting
//!
//! - [`IndexError`]: structural / lookup failures (not-monotonic,
//!   not-unique, missing-label, validation-mismatch).
//! - [`TimedeltaRangeError`] / [`DateRangeError`]: range builder
//!   parse / step / overflow errors.
//!
//! ## Relationship to other crates
//!
//! - **fp-types** supplies [`Scalar`] / [`Timedelta`] /
//!   `format_datetime_ns` primitives.
//! - **fp-frame** stores an `Index` per DataFrame / Series and uses
//!   the alignment algebra here for binary ops.
//! - **fp-join** consumes alignment plans for merge-style joins.

use std::{
    borrow::Cow,
    collections::HashMap,
    fmt,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering as AtomicOrdering},
    },
};

use chrono::Datelike;
use fp_types::{Period, PeriodFreq, Scalar, Timedelta, TimedeltaComponents};
// Dedup / set-op seen-sets key on &IndexLabel and read output order from the
// INPUT scan (first-seen filter / positional bool), never from map iteration —
// so the hasher is observationally invisible. FxHash (rustc-hash, pure safe
// Rust) replaces the std SipHasher on these hot membership maps; public-return
// maps (position_map_first, groupby) keep std HashMap to avoid an API change.
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

/// A total-ordering, hashable `f64` wrapper for use as an index label
/// (br-frankenpandas-i10en). `-0.0` is normalized to `+0.0` and all NaNs
/// collapse to one bucket, so `Eq`/`Hash`/`Ord` are mutually consistent
/// (a == b implies cmp == Equal) — matching pandas' Float64Index. NaN sorts
/// after every finite value (pandas NaN-last).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OrderedF64(pub f64);

impl OrderedF64 {
    #[inline]
    fn canonical_bits(self) -> u64 {
        if self.0.is_nan() {
            f64::NAN.to_bits()
        } else if self.0 == 0.0 {
            0.0_f64.to_bits()
        } else {
            self.0.to_bits()
        }
    }
}

impl PartialEq for OrderedF64 {
    fn eq(&self, other: &Self) -> bool {
        self.canonical_bits() == other.canonical_bits()
    }
}
impl Eq for OrderedF64 {}
impl std::hash::Hash for OrderedF64 {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.canonical_bits().hash(state);
    }
}
impl Ord for OrderedF64 {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        let (a, b) = (self.0, other.0);
        match (a.is_nan(), b.is_nan()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
        }
    }
}
impl PartialOrd for OrderedF64 {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum IndexLabel {
    Int64(i64),
    Utf8(String),
    Timedelta64(i64),
    Datetime64(i64),
    /// pandas Float64Index label (br-frankenpandas-i10en). Placed before
    /// `Null` so the derived cross-variant order keeps every concrete label
    /// before nulls.
    Float64(OrderedF64),
    /// pandas boolean index label.
    Bool(bool),
    /// Typed missing label (br-frankenpandas-joeff): lets value_counts
    /// (dropna=False) and friends keep pandas' distinct None / nan / NaT
    /// buckets instead of collapsing them or colliding with genuine
    /// "None"/"nan" strings. Appended LAST so the derived `Ord` sorts null
    /// labels after every concrete label (pandas NaN-last sort order) without
    /// disturbing the existing cross-variant order. `Eq`/`Hash` are
    /// kind-SENSITIVE (None != nan != NaT), matching `ScalarKey::Null`
    /// bucket identity.
    Null(fp_types::NullKind),
}

impl From<i64> for IndexLabel {
    fn from(value: i64) -> Self {
        Self::Int64(value)
    }
}

impl From<&str> for IndexLabel {
    fn from(value: &str) -> Self {
        Self::Utf8(value.to_owned())
    }
}

impl From<String> for IndexLabel {
    fn from(value: String) -> Self {
        Self::Utf8(value)
    }
}

impl IndexLabel {
    #[must_use]
    fn is_missing(&self) -> bool {
        match self {
            Self::Timedelta64(value) => *value == Timedelta::NAT,
            Self::Datetime64(value) => *value == i64::MIN,
            Self::Float64(v) => v.0.is_nan(),
            Self::Int64(_) | Self::Utf8(_) | Self::Bool(_) => false,
            Self::Null(_) => true,
        }
    }
}

fn index_label_is_truthy(label: &IndexLabel) -> bool {
    if label.is_missing() {
        return false;
    }
    match label {
        IndexLabel::Int64(v) => *v != 0,
        IndexLabel::Float64(v) => v.0 != 0.0,
        IndexLabel::Bool(b) => *b,
        IndexLabel::Utf8(s) => !s.is_empty(),
        IndexLabel::Timedelta64(v) => *v != 0,
        IndexLabel::Datetime64(v) => *v != 0,
        // Unreachable: is_missing() returned true above for every Null.
        IndexLabel::Null(_) => false,
    }
}

impl fmt::Display for IndexLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Int64(v) => write!(f, "{v}"),
            Self::Float64(v) => {
                if v.0.is_nan() {
                    return write!(f, "NaN");
                }
                let t = format!("{}", v.0);
                if t.contains('.') || t.contains('e') || t.contains("inf") {
                    write!(f, "{t}")
                } else {
                    write!(f, "{t}.0")
                }
            }
            Self::Bool(b) => write!(f, "{}", if *b { "True" } else { "False" }),
            Self::Utf8(v) => write!(f, "{v}"),
            Self::Timedelta64(v) => write!(f, "{}", Timedelta::format(*v)),
            Self::Datetime64(v) => write!(f, "{}", format_datetime_ns(*v)),
            // Matches pandas' REPR of missing labels in an index (None / NaN /
            // NaT — note uppercase NaN: the formatter surface, unlike
            // str(nan)=='nan' which astype(str) uses). Verified pandas 2.2.3.
            Self::Null(fp_types::NullKind::Null) => write!(f, "None"),
            Self::Null(fp_types::NullKind::NaN) => write!(f, "NaN"),
            Self::Null(fp_types::NullKind::NaT) => write!(f, "NaT"),
        }
    }
}

pub fn format_datetime_ns(nanos: i64) -> String {
    if nanos == i64::MIN {
        return "NaT".to_owned();
    }
    // `from_timestamp_nanos` decomposes the full i64 nanosecond count with the
    // correct floor semantics for pre-epoch instants (the old manual
    // `nanos / 1e9` + `(nanos % 1e9).unsigned_abs()` mis-decomposed negatives:
    // -0.5s became 1970-01-01 00:00:00.5 instead of 1969-12-31 23:59:59.5).
    let dt = chrono::DateTime::from_timestamp_nanos(nanos);
    let mut rendered = dt.format("%Y-%m-%d %H:%M:%S").to_string();
    // Carry subsecond precision (br-frankenpandas-dt64fmt): the old formatter
    // dropped any fraction, so a Datetime64 column with sub-second nanos lost
    // precision in repr/to_csv. Append the fraction with the SAME trailing-zero
    // trimming `format_naive_datetime` uses, so a Datetime64 value and the Utf8
    // datetime string for the same instant render identically.
    let subsec_nanos = dt.timestamp_subsec_nanos();
    if subsec_nanos != 0 {
        let mut fractional = format!("{subsec_nanos:09}");
        while fractional.ends_with('0') {
            fractional.pop();
        }
        rendered.push('.');
        rendered.push_str(&fractional);
    }
    rendered
}

/// AG-13: Detected sort order of an index's labels.
///
/// Enables adaptive backend selection: binary search for sorted indexes,
/// HashMap fallback for unsorted. Computed lazily via `OnceLock`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortOrder {
    /// Labels are not in any recognized sorted order.
    Unsorted,
    /// All labels are `Int64` and strictly ascending (no duplicates).
    AscendingInt64,
    /// All labels are `Utf8` and strictly ascending (no duplicates).
    AscendingUtf8,
    /// All labels are `Timedelta64` and strictly ascending (no duplicates).
    AscendingTimedelta64,
    /// All labels are `Datetime64` and strictly ascending (no duplicates).
    AscendingDatetime64,
}

/// Which set operation a two-pointer sorted merge should emit
/// (br-frankenpandas-idxdup). Both inputs are strictly ascending and unique.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetMergeKind {
    /// Keep `self` labels that also appear in `other`.
    Intersection,
    /// Keep `self` labels that do NOT appear in `other`.
    Difference,
}

/// Detect the sort order of the label slice.
fn detect_sort_order(labels: &[IndexLabel]) -> SortOrder {
    if labels.len() <= 1 {
        return match labels.first() {
            Some(IndexLabel::Int64(_)) | None => SortOrder::AscendingInt64,
            Some(IndexLabel::Utf8(_)) => SortOrder::AscendingUtf8,
            Some(IndexLabel::Timedelta64(_)) => SortOrder::AscendingTimedelta64,
            Some(IndexLabel::Datetime64(_)) => SortOrder::AscendingDatetime64,
            // Float64/Bool/Null labels use the general (non-typed) backend.
            Some(IndexLabel::Float64(_) | IndexLabel::Bool(_) | IndexLabel::Null(_)) => {
                SortOrder::Unsorted
            }
        };
    }

    // Check if all Int64 and strictly ascending.
    let all_int = labels.iter().all(|l| matches!(l, IndexLabel::Int64(_)));
    if all_int {
        let is_sorted = labels.windows(2).all(|w| {
            if let (IndexLabel::Int64(a), IndexLabel::Int64(b)) = (&w[0], &w[1]) {
                a < b
            } else {
                false
            }
        });
        if is_sorted {
            return SortOrder::AscendingInt64;
        }
    }

    // Check if all Utf8 and strictly ascending.
    let all_utf8 = labels.iter().all(|l| matches!(l, IndexLabel::Utf8(_)));
    if all_utf8 {
        let is_sorted = labels.windows(2).all(|w| {
            if let (IndexLabel::Utf8(a), IndexLabel::Utf8(b)) = (&w[0], &w[1]) {
                a < b
            } else {
                false
            }
        });
        if is_sorted {
            return SortOrder::AscendingUtf8;
        }
    }

    // Check if all Timedelta64 and strictly ascending.
    let all_td = labels
        .iter()
        .all(|l| matches!(l, IndexLabel::Timedelta64(_)));
    if all_td {
        let is_sorted = labels.windows(2).all(|w| {
            if let (IndexLabel::Timedelta64(a), IndexLabel::Timedelta64(b)) = (&w[0], &w[1]) {
                a < b
            } else {
                false
            }
        });
        if is_sorted {
            return SortOrder::AscendingTimedelta64;
        }
    }

    // Check if all Datetime64 and strictly ascending.
    let all_dt = labels
        .iter()
        .all(|l| matches!(l, IndexLabel::Datetime64(_)));
    if all_dt {
        let is_sorted = labels.windows(2).all(|w| {
            if let (IndexLabel::Datetime64(a), IndexLabel::Datetime64(b)) = (&w[0], &w[1]) {
                a < b
            } else {
                false
            }
        });
        if is_sorted {
            return SortOrder::AscendingDatetime64;
        }
    }

    SortOrder::Unsorted
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DuplicateKeep {
    First,
    Last,
    None,
}

static INDEX_LABEL_ID_COUNTER: AtomicU64 = AtomicU64::new(1);
static INDEX_LABEL_EQUALITY_CACHE: OnceLock<Mutex<FxHashMap<(u64, u64), bool>>> = OnceLock::new();

const INDEX_LABEL_EQUALITY_CACHE_MAX: usize = 4096;

fn next_index_label_identity() -> u64 {
    INDEX_LABEL_ID_COUNTER.fetch_add(1, AtomicOrdering::Relaxed)
}

/// Shared contiguous-Utf8 label backing: a byte buffer + `n+1` offsets, row `i`
/// being `bytes[offsets[i]..offsets[i+1]]` (br-frankenpandas-nbspq).
type Utf8LabelBacking = (Arc<[u8]>, Arc<[usize]>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Int64UnitRangeLabels {
    start: i64,
    len: usize,
}

impl Int64UnitRangeLabels {
    fn new(start: i64, len: usize) -> Option<Self> {
        if len > 0 {
            let last_offset = i64::try_from(len.checked_sub(1)?).ok()?;
            start.checked_add(last_offset)?;
        }
        Some(Self { start, len })
    }

    fn materialize(self) -> Vec<IndexLabel> {
        let mut labels = Vec::with_capacity(self.len);
        for offset in 0..self.len {
            let offset = i64::try_from(offset).expect("validated Int64 unit range length");
            labels.push(IndexLabel::Int64(
                self.start
                    .checked_add(offset)
                    .expect("validated Int64 unit range end"),
            ));
        }
        labels
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Int64AffineLabels {
    start: i64,
    step: i64,
    len: usize,
}

impl Int64AffineLabels {
    fn new(start: i64, step: i64, len: usize) -> Option<Self> {
        if len > 1 && step == 0 {
            return None;
        }
        if len > 0 {
            let last_index = i64::try_from(len.checked_sub(1)?).ok()?;
            let span = step.checked_mul(last_index)?;
            start.checked_add(span)?;
        }
        Some(Self { start, step, len })
    }

    fn materialize(self) -> Vec<IndexLabel> {
        let mut labels = Vec::with_capacity(self.len);
        let mut value = self.start;
        for offset in 0..self.len {
            labels.push(IndexLabel::Int64(value));
            if offset + 1 < self.len {
                value = value
                    .checked_add(self.step)
                    .expect("validated Int64 affine range end");
            }
        }
        labels
    }

    fn materialize_i64(self) -> Vec<i64> {
        let mut labels = Vec::with_capacity(self.len);
        let mut value = self.start;
        for offset in 0..self.len {
            labels.push(value);
            if offset + 1 < self.len {
                value = value
                    .checked_add(self.step)
                    .expect("validated Int64 affine range end");
            }
        }
        labels
    }

    fn position(self, target: i64) -> Option<usize> {
        if self.len == 0 {
            return None;
        }
        if self.step == 0 {
            return (self.len == 1 && target == self.start).then_some(0);
        }
        let offset = target.checked_sub(self.start)?;
        if offset.checked_rem(self.step)? != 0 {
            return None;
        }
        let pos = offset.checked_div(self.step)?;
        let pos = usize::try_from(pos).ok()?;
        (pos < self.len).then_some(pos)
    }

    fn equals_slice(self, labels: &[IndexLabel]) -> bool {
        labels.len() == self.len
            && labels.iter().enumerate().all(|(offset, label)| {
                let Ok(offset) = i64::try_from(offset) else {
                    return false;
                };
                let Some(delta) = self.step.checked_mul(offset) else {
                    return false;
                };
                matches!(
                    label,
                    IndexLabel::Int64(value)
                        if self.start.checked_add(delta).is_some_and(|expected| *value == expected)
                )
            })
    }
}

#[derive(Debug, Clone)]
struct Int64StridedLabels {
    values: Arc<Vec<i64>>,
    start: usize,
    step: usize,
    len: usize,
}

impl Int64StridedLabels {
    fn new(values: Arc<Vec<i64>>, start: usize, step: usize, len: usize) -> Option<Self> {
        if len > 1 && step == 0 {
            return None;
        }
        if len > 0 {
            let last = start.checked_add(step.checked_mul(len.checked_sub(1)?)?)?;
            if last >= values.len() {
                return None;
            }
        }
        Some(Self {
            values,
            start,
            step,
            len,
        })
    }

    fn materialize(self) -> Vec<IndexLabel> {
        let mut labels = Vec::with_capacity(self.len);
        let mut pos = self.start;
        for offset in 0..self.len {
            labels.push(IndexLabel::Int64(self.values[pos]));
            if offset + 1 < self.len {
                pos = pos
                    .checked_add(self.step)
                    .expect("validated Int64 strided range end");
            }
        }
        labels
    }

    fn materialize_i64(self) -> Vec<i64> {
        let mut labels = Vec::with_capacity(self.len);
        let mut pos = self.start;
        for offset in 0..self.len {
            labels.push(self.values[pos]);
            if offset + 1 < self.len {
                pos = pos
                    .checked_add(self.step)
                    .expect("validated Int64 strided range end");
            }
        }
        labels
    }
}

#[derive(Debug, Clone)]
struct MaterializedLabelSlice {
    labels: Arc<Vec<IndexLabel>>,
    start: usize,
    len: usize,
}

impl MaterializedLabelSlice {
    fn new(labels: Arc<Vec<IndexLabel>>, start: usize, len: usize) -> Option<Self> {
        start.checked_add(len).filter(|end| *end <= labels.len())?;
        Some(Self { labels, start, len })
    }

    fn as_slice(&self) -> &[IndexLabel] {
        &self.labels[self.start..self.start + self.len]
    }
}

struct IndexLabels {
    /// Shared immutable label vector (br-frankenpandas-idxclone). Behind `Arc`
    /// so cloning an `Index` is an O(1) refcount bump instead of an O(n)
    /// `Vec<IndexLabel>` deep copy — the dominant cost of same-index binary ops
    /// (`a + b` re-uses the operand index). Set once, never mutated, so sharing
    /// is observationally identical to a private copy.
    materialized: OnceLock<Arc<Vec<IndexLabel>>>,
    materialized_slice: Option<Arc<MaterializedLabelSlice>>,
    int64_unit_range: Option<Int64UnitRangeLabels>,
    int64_affine: Option<Int64AffineLabels>,
    int64_strided: Option<Int64StridedLabels>,
    /// Lazy typed Int64 backing (br-frankenpandas-dxqpm). `Some(values)` once
    /// computed means every label is `IndexLabel::Int64` and `values` is the
    /// raw `i64` view; `None` once computed means the labels are not all
    /// Int64. Pre-seeded by typed constructors so gathers/clones/drops of
    /// Int64-labelled indexes stay on contiguous `i64` storage instead of the
    /// 32 B enum representation.
    int64_typed: OnceLock<Option<Arc<Vec<i64>>>>,
    /// Lazy contiguous-Utf8 backing (br-frankenpandas-nbspq): `Some((bytes,
    /// offsets))` means every label is `IndexLabel::Utf8(bytes[off[i]..off[i+1]])`
    /// (valid UTF-8 by construction). Pre-seeded by `new_utf8_contiguous` so a
    /// string-keyed result index (groupby keys, sort_values, set-ops) avoids the
    /// per-label `String` alloc + `from_utf8` re-validation until something
    /// actually needs the `Vec<IndexLabel>` view.
    utf8_contiguous: Option<Utf8LabelBacking>,
}

impl IndexLabels {
    fn new(labels: Vec<IndexLabel>) -> Self {
        let materialized = OnceLock::new();
        let _ = materialized.set(Arc::new(labels));
        Self {
            materialized,
            materialized_slice: None,
            int64_unit_range: None,
            int64_affine: None,
            int64_strided: None,
            int64_typed: OnceLock::new(),
            utf8_contiguous: None,
        }
    }

    fn new_int64_unit_range(start: i64, len: usize) -> Option<Self> {
        Some(Self {
            materialized: OnceLock::new(),
            materialized_slice: None,
            int64_unit_range: Some(Int64UnitRangeLabels::new(start, len)?),
            int64_affine: None,
            int64_strided: None,
            int64_typed: OnceLock::new(),
            utf8_contiguous: None,
        })
    }

    fn new_int64_affine(start: i64, step: i64, len: usize) -> Option<Self> {
        if step == 1 {
            return Self::new_int64_unit_range(start, len);
        }
        Some(Self {
            materialized: OnceLock::new(),
            materialized_slice: None,
            int64_unit_range: None,
            int64_affine: Some(Int64AffineLabels::new(start, step, len)?),
            int64_strided: None,
            int64_typed: OnceLock::new(),
            utf8_contiguous: None,
        })
    }

    fn new_int64_strided(
        values: Arc<Vec<i64>>,
        start: usize,
        step: usize,
        len: usize,
    ) -> Option<Self> {
        Some(Self {
            materialized: OnceLock::new(),
            materialized_slice: None,
            int64_unit_range: None,
            int64_affine: None,
            int64_strided: Some(Int64StridedLabels::new(values, start, step, len)?),
            int64_typed: OnceLock::new(),
            utf8_contiguous: None,
        })
    }

    fn new_int64_values(values: Arc<Vec<i64>>) -> Self {
        let int64_typed = OnceLock::new();
        let _ = int64_typed.set(Some(values));
        Self {
            materialized: OnceLock::new(),
            materialized_slice: None,
            int64_unit_range: None,
            int64_affine: None,
            int64_strided: None,
            int64_typed,
            utf8_contiguous: None,
        }
    }

    fn new_utf8_contiguous(bytes: Arc<[u8]>, offsets: Arc<[usize]>) -> Self {
        debug_assert!(!offsets.is_empty());
        debug_assert_eq!(*offsets.last().expect("non-empty"), bytes.len());
        Self {
            materialized: OnceLock::new(),
            materialized_slice: None,
            int64_unit_range: None,
            int64_affine: None,
            int64_strided: None,
            int64_typed: OnceLock::new(),
            utf8_contiguous: Some((bytes, offsets)),
        }
    }

    fn as_slice(&self) -> &[IndexLabel] {
        if let Some(slice) = &self.materialized_slice {
            return slice.as_slice();
        }
        self.materialized
            .get_or_init(|| {
                if let Some(range) = self.int64_unit_range {
                    return Arc::new(range.materialize());
                }
                if let Some(range) = self.int64_affine {
                    return Arc::new(range.materialize());
                }
                if let Some((bytes, offsets)) = &self.utf8_contiguous {
                    return Arc::new(
                        offsets
                            .windows(2)
                            .map(|w| {
                                IndexLabel::Utf8(
                                    std::str::from_utf8(&bytes[w[0]..w[1]])
                                        .expect("contiguous utf8 index buffer is valid")
                                        .to_owned(),
                                )
                            })
                            .collect(),
                    );
                }
                if let Some(strided) = self.int64_strided.clone() {
                    return Arc::new(strided.materialize());
                }
                let values = self
                    .int64_typed
                    .get()
                    .and_then(Option::as_ref)
                    .expect("lazy index labels require a typed or range backing");
                Arc::new(values.iter().copied().map(IndexLabel::Int64).collect())
            })
            .as_slice()
    }

    fn len(&self) -> usize {
        if let Some(slice) = &self.materialized_slice {
            return slice.len;
        }
        if let Some(range) = self.int64_unit_range {
            return range.len;
        }
        if let Some(range) = self.int64_affine {
            return range.len;
        }
        if let Some(strided) = &self.int64_strided {
            return strided.len;
        }
        if let Some((_, offsets)) = &self.utf8_contiguous {
            return offsets.len() - 1;
        }
        if let Some(labels) = self.materialized.get() {
            return labels.len();
        }
        if let Some(Some(values)) = self.int64_typed.get() {
            return values.len();
        }
        self.as_slice().len()
    }

    fn slice(&self, start: usize, len: usize) -> Self {
        let total_len = self.len();
        let start = start.min(total_len);
        let end = start.saturating_add(len).min(total_len);
        let len = end - start;

        if let Some(range) = self.int64_unit_range {
            let offset = i64::try_from(start).expect("start within index length");
            if let Some(next_start) = range.start.checked_add(offset)
                && let Some(labels) = Self::new_int64_unit_range(next_start, len)
            {
                return labels;
            }
        }

        if let Some(range) = self.int64_affine {
            let offset = i64::try_from(start).expect("start within index length");
            if let Some(delta) = range.step.checked_mul(offset)
                && let Some(next_start) = range.start.checked_add(delta)
                && let Some(labels) = Self::new_int64_affine(next_start, range.step, len)
            {
                return labels;
            }
        }

        if let Some(strided) = &self.int64_strided
            && let Some(offset) = strided.step.checked_mul(start)
            && let Some(next_start) = strided.start.checked_add(offset)
            && let Some(labels) =
                Self::new_int64_strided(Arc::clone(&strided.values), next_start, strided.step, len)
        {
            return labels;
        }

        if let Some(Some(values)) = self.int64_typed.get()
            && let Some(labels) = Self::new_int64_strided(Arc::clone(values), start, 1, len)
        {
            return labels;
        }

        if let Some(slice) = &self.materialized_slice
            && let Some(next_start) = slice.start.checked_add(start)
            && let Some(view) =
                MaterializedLabelSlice::new(Arc::clone(&slice.labels), next_start, len)
        {
            return Self {
                materialized: OnceLock::new(),
                materialized_slice: Some(Arc::new(view)),
                int64_unit_range: None,
                int64_affine: None,
                int64_strided: None,
                int64_typed: OnceLock::new(),
                utf8_contiguous: None,
            };
        }

        if let Some(labels) = self.materialized.get()
            && let Some(view) = MaterializedLabelSlice::new(Arc::clone(labels), start, len)
        {
            return Self {
                materialized: OnceLock::new(),
                materialized_slice: Some(Arc::new(view)),
                int64_unit_range: None,
                int64_affine: None,
                int64_strided: None,
                int64_typed: OnceLock::new(),
                utf8_contiguous: None,
            };
        }

        Self::new(self.as_slice()[start..end].to_vec())
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn int64_unit_range(&self) -> Option<Int64UnitRangeLabels> {
        self.int64_unit_range
    }

    fn int64_affine_range(&self) -> Option<Int64AffineLabels> {
        self.int64_unit_range
            .map(|range| Int64AffineLabels {
                start: range.start,
                step: 1,
                len: range.len,
            })
            .or(self.int64_affine)
    }

    /// The raw `i64` view of an all-Int64 label vector, computing and caching
    /// it on first request. `None` means at least one label is not Int64.
    fn int64_view(&self) -> Option<Arc<Vec<i64>>> {
        self.int64_typed
            .get_or_init(|| {
                if let Some(range) = self.int64_unit_range {
                    let mut values = Vec::with_capacity(range.len);
                    for offset in 0..range.len {
                        let offset =
                            i64::try_from(offset).expect("validated Int64 unit range length");
                        values.push(
                            range
                                .start
                                .checked_add(offset)
                                .expect("validated Int64 unit range end"),
                        );
                    }
                    return Some(Arc::new(values));
                }
                if let Some(range) = self.int64_affine {
                    return Some(Arc::new(range.materialize_i64()));
                }
                if let Some(strided) = self.int64_strided.clone() {
                    return Some(Arc::new(strided.materialize_i64()));
                }
                if let Some(slice) = &self.materialized_slice {
                    let labels = slice.as_slice();
                    let mut values = Vec::with_capacity(labels.len());
                    for label in labels {
                        match label {
                            IndexLabel::Int64(value) => values.push(*value),
                            _ => return None,
                        }
                    }
                    return Some(Arc::new(values));
                }
                let labels = self.materialized.get()?;
                let mut values = Vec::with_capacity(labels.len());
                for label in labels.iter() {
                    match label {
                        IndexLabel::Int64(value) => values.push(*value),
                        _ => return None,
                    }
                }
                Some(Arc::new(values))
            })
            .clone()
    }

    /// The cached `i64` view if it has already been computed (never computes).
    /// Outer `None` = not yet computed; `Some(None)` = known non-Int64.
    fn cached_int64_view(&self) -> Option<Option<Arc<Vec<i64>>>> {
        self.int64_typed.get().cloned()
    }
}

impl Clone for IndexLabels {
    fn clone(&self) -> Self {
        let int64_typed = OnceLock::new();
        if let Some(view) = self.int64_typed.get() {
            let _ = int64_typed.set(view.clone());
        }
        let materialized = OnceLock::new();
        // A unit-range, typed Int64, or contiguous-Utf8 backing can regenerate
        // the label vector on demand, so skip the O(n) Vec<IndexLabel> deep clone.
        let has_lazy_backing = self.int64_unit_range.is_some()
            || self.int64_affine.is_some()
            || self.int64_strided.is_some()
            || self.materialized_slice.is_some()
            || self.utf8_contiguous.is_some()
            || matches!(int64_typed.get(), Some(Some(_)));
        if !has_lazy_backing && let Some(labels) = self.materialized.get() {
            let _ = materialized.set(labels.clone());
        }
        Self {
            materialized,
            materialized_slice: self.materialized_slice.clone(),
            int64_unit_range: self.int64_unit_range,
            int64_affine: self.int64_affine,
            int64_strided: self.int64_strided.clone(),
            int64_typed,
            utf8_contiguous: self.utf8_contiguous.clone(),
        }
    }
}

impl Default for IndexLabels {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl fmt::Debug for IndexLabels {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_slice().fmt(f)
    }
}

impl PartialEq for IndexLabels {
    fn eq(&self, other: &Self) -> bool {
        match (self.int64_affine_range(), other.int64_affine_range()) {
            (Some(left), Some(right)) => left == right,
            (Some(range), None) => range.equals_slice(other.as_slice()),
            (None, Some(range)) => range.equals_slice(self.as_slice()),
            (None, None) => self.as_slice() == other.as_slice(),
        }
    }
}

impl Eq for IndexLabels {}

impl std::ops::Deref for IndexLabels {
    type Target = [IndexLabel];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl<'a> IntoIterator for &'a IndexLabels {
    type Item = &'a IndexLabel;
    type IntoIter = std::slice::Iter<'a, IndexLabel>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}

impl Serialize for IndexLabels {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.as_slice().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for IndexLabels {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Vec::<IndexLabel>::deserialize(deserializer).map(Self::new)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Index {
    #[serde(default)]
    labels: IndexLabels,
    /// Optional name for the index (matches pandas `Index.name`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    /// Runtime-only immutable identity for this label vector lineage.
    #[serde(skip, default = "next_index_label_identity")]
    label_identity: u64,
    #[serde(skip)]
    duplicate_cache: OnceLock<bool>,
    /// AG-13: Cached sort order for adaptive backend selection.
    #[serde(skip)]
    sort_order_cache: OnceLock<SortOrder>,
    /// Runtime-only cache for labels-derived AACE semantic fingerprints.
    #[serde(skip)]
    semantic_fingerprint_cache: OnceLock<String>,
}

impl PartialEq for Index {
    fn eq(&self, other: &Self) -> bool {
        self.labels_equal(other)
    }
}

impl Eq for Index {}

fn detect_duplicates(labels: &[IndexLabel]) -> bool {
    let mut seen = FxHashMap::<&IndexLabel, ()>::default();
    for label in labels {
        if seen.insert(label, ()).is_some() {
            return true;
        }
    }
    false
}

fn ordered_label_identity_pair(left: u64, right: u64) -> (u64, u64) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

impl Index {
    #[must_use]
    pub fn new(labels: Vec<IndexLabel>) -> Self {
        Self {
            labels: IndexLabels::new(labels),
            name: None,
            label_identity: next_index_label_identity(),
            duplicate_cache: OnceLock::new(),
            sort_order_cache: OnceLock::new(),
            semantic_fingerprint_cache: OnceLock::new(),
        }
    }

    fn labels_equal(&self, other: &Self) -> bool {
        if self.label_identity == other.label_identity {
            return true;
        }

        let key = ordered_label_identity_pair(self.label_identity, other.label_identity);
        let cache = INDEX_LABEL_EQUALITY_CACHE.get_or_init(|| Mutex::new(FxHashMap::default()));
        if let Some(equal) = cache
            .lock()
            .expect("index label equality cache poisoned")
            .get(&key)
            .copied()
        {
            return equal;
        }

        let equal = self.labels == other.labels;
        let mut guard = cache.lock().expect("index label equality cache poisoned");
        if guard.len() >= INDEX_LABEL_EQUALITY_CACHE_MAX {
            guard.clear();
        }
        guard.insert(key, equal);
        equal
    }

    /// Construct an index whose caller has already proven all labels unique.
    ///
    /// This preserves the public `Index::new` surface while letting alignment
    /// builders carry their uniqueness proof into the runtime duplicate cache.
    #[must_use]
    #[doc(hidden)]
    pub fn new_known_unique(labels: Vec<IndexLabel>) -> Self {
        debug_assert!(!detect_duplicates(&labels));
        let index = Self::new(labels);
        let _ = index.duplicate_cache.set(false);
        index
    }

    /// Construct an index whose labels are the dense unit range
    /// `start..start+len`, without allocating the label vector until a caller
    /// asks for label materialization.
    #[must_use]
    #[doc(hidden)]
    pub fn new_known_unique_int64_unit_range(start: i64, len: usize) -> Self {
        let labels = IndexLabels::new_int64_unit_range(start, len)
            .expect("validated Int64 unit range bounds");
        let index = Self {
            labels,
            name: None,
            label_identity: next_index_label_identity(),
            duplicate_cache: OnceLock::new(),
            sort_order_cache: OnceLock::new(),
            semantic_fingerprint_cache: OnceLock::new(),
        };
        let _ = index.duplicate_cache.set(false);
        let _ = index.sort_order_cache.set(SortOrder::AscendingInt64);
        index
    }

    /// Construct an index whose labels are the affine Int64 sequence
    /// `start + i * step`, without allocating the label vector until a caller
    /// asks for label materialization.
    #[must_use]
    #[doc(hidden)]
    pub fn new_known_unique_int64_affine_range(start: i64, step: i64, len: usize) -> Option<Self> {
        let labels = IndexLabels::new_int64_affine(start, step, len)?;
        let index = Self {
            labels,
            name: None,
            label_identity: next_index_label_identity(),
            duplicate_cache: OnceLock::new(),
            sort_order_cache: OnceLock::new(),
            semantic_fingerprint_cache: OnceLock::new(),
        };
        let _ = index.duplicate_cache.set(false);
        if len <= 1 || step > 0 {
            let _ = index.sort_order_cache.set(SortOrder::AscendingInt64);
        }
        Some(index)
    }

    #[must_use]
    pub fn from_i64(values: Vec<i64>) -> Self {
        Self::from_i64_values(values)
    }

    /// Construct an index over Int64 labels backed by a contiguous `Vec<i64>`
    /// (br-frankenpandas-dxqpm). Label materialization into `IndexLabel`s is
    /// deferred until a caller asks for `labels()`; clones of the index share
    /// the typed backing instead of deep-copying the enum vector.
    #[must_use]
    #[doc(hidden)]
    pub fn from_i64_values(values: Vec<i64>) -> Self {
        Self {
            labels: IndexLabels::new_int64_values(Arc::new(values)),
            name: None,
            label_identity: next_index_label_identity(),
            duplicate_cache: OnceLock::new(),
            sort_order_cache: OnceLock::new(),
            semantic_fingerprint_cache: OnceLock::new(),
        }
    }

    /// Construct an Int64-labelled index as a strided view over an existing
    /// typed backing. Unlike affine ranges, the source values may be duplicated
    /// or unsorted, so uniqueness and sort caches are intentionally not seeded.
    #[must_use]
    #[doc(hidden)]
    pub fn from_i64_strided_values(
        values: Arc<Vec<i64>>,
        start: usize,
        step: usize,
        len: usize,
    ) -> Option<Self> {
        Some(Self {
            labels: IndexLabels::new_int64_strided(values, start, step, len)?,
            name: None,
            label_identity: next_index_label_identity(),
            duplicate_cache: OnceLock::new(),
            sort_order_cache: OnceLock::new(),
            semantic_fingerprint_cache: OnceLock::new(),
        })
    }

    /// Construct an index over Utf8 labels backed by a contiguous byte buffer +
    /// offsets (br-frankenpandas-nbspq). Label materialization into
    /// `IndexLabel::Utf8` is deferred until `labels()` is asked for; clones share
    /// the `Arc` backing. Caller guarantees `bytes` is valid UTF-8 and
    /// `offsets` holds `n+1` ascending entries ending at `bytes.len()`.
    #[must_use]
    #[doc(hidden)]
    pub fn from_utf8_contiguous(bytes: Arc<[u8]>, offsets: Arc<[usize]>) -> Self {
        Self {
            labels: IndexLabels::new_utf8_contiguous(bytes, offsets),
            name: None,
            label_identity: next_index_label_identity(),
            duplicate_cache: OnceLock::new(),
            sort_order_cache: OnceLock::new(),
            semantic_fingerprint_cache: OnceLock::new(),
        }
    }

    /// Raw `i64` view of an all-Int64 label vector, computing and caching it
    /// on first request. `None` means at least one label is not Int64.
    #[must_use]
    #[doc(hidden)]
    pub fn int64_label_values(&self) -> Option<Arc<Vec<i64>>> {
        self.labels.int64_view()
    }

    /// The cached `i64` label view if already computed (never computes).
    /// Outer `None` = not yet computed; `Some(None)` = known non-Int64.
    #[must_use]
    #[doc(hidden)]
    pub fn cached_int64_label_values(&self) -> Option<Option<Arc<Vec<i64>>>> {
        self.labels.cached_int64_view()
    }

    #[must_use]
    pub fn from_utf8(values: Vec<String>) -> Self {
        Self::new(values.into_iter().map(IndexLabel::from).collect())
    }

    #[must_use]
    pub fn from_timedelta64(nanos: Vec<i64>) -> Self {
        Self::new(nanos.into_iter().map(IndexLabel::Timedelta64).collect())
    }

    #[must_use]
    pub fn from_datetime64(nanos: Vec<i64>) -> Self {
        Self::new(nanos.into_iter().map(IndexLabel::Datetime64).collect())
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.labels.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.labels.is_empty()
    }

    #[must_use]
    pub fn labels(&self) -> &[IndexLabel] {
        self.labels.as_slice()
    }

    #[must_use]
    #[doc(hidden)]
    pub fn int64_unit_range_labels(&self) -> Option<(i64, usize)> {
        self.labels
            .int64_unit_range()
            .map(|range| (range.start, range.len))
    }

    #[must_use]
    pub fn semantic_labels_fingerprint_with<F>(&self, compute: F) -> String
    where
        F: FnOnce(&[IndexLabel]) -> String,
    {
        self.semantic_fingerprint_cache
            .get_or_init(|| compute(self.labels()))
            .clone()
    }

    /// Return the index name (matches `pd.Index.name`).
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Return a new index with the given name (matches `pd.Index.set_names`).
    #[must_use]
    pub fn set_names(&self, name: Option<&str>) -> Self {
        let mut idx = self.clone();
        idx.name = name.map(String::from);
        idx
    }

    /// Alias for `set_names` — set the index name, returning a new `Index`.
    #[must_use]
    pub fn set_name(&self, name: &str) -> Self {
        self.set_names(Some(name))
    }

    /// Return a list of index names.
    ///
    /// Matches `pd.Index.names`. For a flat (non-MultiIndex) index this returns
    /// a single-element list with the current name (or `None`).
    #[must_use]
    pub fn names(&self) -> Vec<Option<String>> {
        vec![self.name.clone()]
    }

    /// Set names from a list.
    ///
    /// Matches `pd.Index.set_names([name])`. For flat index only the first
    /// element is used. Panics if the list is empty.
    #[must_use]
    pub fn set_names_list(&self, names: &[Option<&str>]) -> Self {
        assert!(
            !names.is_empty(),
            "set_names_list requires at least one name"
        );
        self.set_names(names[0])
    }

    /// Return the index as-is (flat index identity).
    ///
    /// Matches `pd.Index.to_flat_index()`. For a non-MultiIndex this is a
    /// no-op that returns a clone. For a MultiIndex it would convert tuples
    /// to flat labels.
    #[must_use]
    pub fn to_flat_index(&self) -> Self {
        self.clone()
    }

    /// Return a new index with the name cleared.
    #[must_use]
    pub fn rename_index(&self, name: Option<&str>) -> Self {
        self.set_names(name)
    }

    /// Internal: propagate this index's name onto a newly created index.
    fn propagate_name(&self, mut other: Self) -> Self {
        other.name.clone_from(&self.name);
        other
    }

    /// Internal: if both indexes share the same name, return it; otherwise None.
    /// Matches pandas behavior for binary set operations.
    fn shared_name(&self, other: &Self) -> Option<String> {
        if self.name == other.name {
            self.name.clone()
        } else {
            None
        }
    }

    #[must_use]
    pub fn has_duplicates(&self) -> bool {
        if self.labels.int64_affine_range().is_some() {
            return false;
        }
        *self.duplicate_cache.get_or_init(|| {
            // Every `SortOrder::Ascending*` variant is STRICTLY ascending
            // (`detect_sort_order` rejects equal neighbours with `a < b`), so a
            // recognized sort order proves uniqueness with zero hashing
            // (br-frankenpandas-idxdup). `sort_order()` is a single linear pass
            // (itself cached and reused by the binary-search backends), far
            // cheaper than the FxHashMap insert-per-label below; only genuinely
            // unsorted indexes fall through to it.
            if !matches!(self.sort_order(), SortOrder::Unsorted) {
                return false;
            }
            detect_duplicates(self.labels())
        })
    }

    /// Whether all index labels are unique.
    ///
    /// Matches `pd.Index.is_unique`.
    #[must_use]
    pub fn is_unique(&self) -> bool {
        !self.has_duplicates()
    }

    /// Get the position (integer location) of a label.
    ///
    /// Matches `pd.Index.get_loc(label)`.
    #[must_use]
    pub fn get_loc(&self, label: &IndexLabel) -> Option<usize> {
        self.position(label)
    }

    /// AG-13: Lazily detect and cache the sort order of this index.
    #[must_use]
    fn sort_order(&self) -> SortOrder {
        if self
            .labels
            .int64_affine_range()
            .is_some_and(|range| range.len <= 1 || range.step > 0)
        {
            return SortOrder::AscendingInt64;
        }
        *self
            .sort_order_cache
            .get_or_init(|| detect_sort_order(self.labels()))
    }

    /// Returns `true` if this index is sorted (strictly ascending, no duplicates).
    #[must_use]
    pub fn is_sorted(&self) -> bool {
        !matches!(self.sort_order(), SortOrder::Unsorted)
    }

    /// AG-13: Adaptive position lookup.
    ///
    /// For sorted `Int64` or `Utf8` indexes, uses binary search (O(log n)).
    /// For unsorted indexes, falls back to linear scan (O(n)).
    #[must_use]
    pub fn position(&self, needle: &IndexLabel) -> Option<usize> {
        if let (Some(range), IndexLabel::Int64(target)) = (self.labels.int64_affine_range(), needle)
        {
            return range.position(*target);
        }
        match self.sort_order() {
            SortOrder::AscendingInt64 => {
                if let IndexLabel::Int64(target) = needle {
                    self.labels
                        .binary_search_by(|label| {
                            if let IndexLabel::Int64(v) = label {
                                v.cmp(target)
                            } else {
                                std::cmp::Ordering::Less
                            }
                        })
                        .ok()
                } else {
                    None // Type mismatch: no Int64 label can match a Utf8 needle
                }
            }
            SortOrder::AscendingUtf8 => {
                if let IndexLabel::Utf8(target) = needle {
                    self.labels
                        .binary_search_by(|label| {
                            if let IndexLabel::Utf8(v) = label {
                                v.as_str().cmp(target.as_str())
                            } else {
                                std::cmp::Ordering::Less
                            }
                        })
                        .ok()
                } else {
                    None
                }
            }
            SortOrder::AscendingTimedelta64 => {
                if let IndexLabel::Timedelta64(target) = needle {
                    self.labels
                        .binary_search_by(|label| {
                            if let IndexLabel::Timedelta64(v) = label {
                                v.cmp(target)
                            } else {
                                std::cmp::Ordering::Less
                            }
                        })
                        .ok()
                } else {
                    None
                }
            }
            SortOrder::AscendingDatetime64 => {
                if let IndexLabel::Datetime64(target) = needle {
                    self.labels
                        .binary_search_by(|label| {
                            if let IndexLabel::Datetime64(v) = label {
                                v.cmp(target)
                            } else {
                                std::cmp::Ordering::Less
                            }
                        })
                        .ok()
                } else {
                    None
                }
            }
            SortOrder::Unsorted => self.labels.iter().position(|label| label == needle),
        }
    }

    #[must_use]
    pub fn position_map_first(&self) -> HashMap<IndexLabel, usize> {
        let mut positions = HashMap::with_capacity(self.labels.len());
        for (idx, label) in self.labels.iter().enumerate() {
            positions.entry(label.clone()).or_insert(idx);
        }
        positions
    }

    fn position_map_first_ref(&self) -> FxHashMap<&IndexLabel, usize> {
        let mut positions =
            FxHashMap::with_capacity_and_hasher(self.labels.len(), Default::default());
        for (idx, label) in self.labels.iter().enumerate() {
            positions.entry(label).or_insert(idx);
        }
        positions
    }

    // ── Pandas Index Model: lookup and membership ──────────────────────

    #[must_use]
    pub fn contains(&self, label: &IndexLabel) -> bool {
        self.position(label).is_some()
    }

    #[must_use]
    pub fn get_indexer(&self, target: &Index) -> Vec<Option<usize>> {
        // When `self` is strictly ascending (any SortOrder::Ascending* ⟹
        // globally IndexLabel::Ord-sorted and unique) we can resolve target
        // positions without building the O(n) FxHashMap of `self`
        // (br-frankenpandas-idxdup):
        //   * target also sorted  ⇒ one two-pointer merge, O(n+m), no hashing;
        //   * target unsorted     ⇒ binary-search each label, O(m log n).
        // Both yield the same first-occurrence position the hash path returns
        // (uniqueness makes "first" the only one); unsorted `self` keeps the
        // hash path so a per-label scan never degrades to O(n·m).
        if !matches!(self.sort_order(), SortOrder::Unsorted) {
            let labels = self.labels();
            let targets = target.labels();
            if !matches!(target.sort_order(), SortOrder::Unsorted) {
                let mut out = Vec::with_capacity(targets.len());
                let mut i = 0usize;
                for label in targets {
                    while i < labels.len() && labels[i] < *label {
                        i += 1;
                    }
                    if i < labels.len() && labels[i] == *label {
                        out.push(Some(i));
                    } else {
                        out.push(None);
                    }
                }
                return out;
            }
            return targets.iter().map(|label| self.position(label)).collect();
        }
        let map = self.position_map_first_ref();
        target
            .labels
            .iter()
            .map(|label| map.get(label).copied())
            .collect()
    }

    #[must_use]
    pub fn isin(&self, values: &[IndexLabel]) -> Vec<bool> {
        let set: FxHashMap<&IndexLabel, ()> = values.iter().map(|v| (v, ())).collect();
        self.labels.iter().map(|l| set.contains_key(l)).collect()
    }

    // ── Pandas Index Model: deduplication ──────────────────────────────

    #[must_use]
    pub fn is_monotonic_increasing(&self) -> bool {
        if self.labels.len() <= 1 {
            return true;
        }
        for pair in self.labels.windows(2) {
            if pair[0] > pair[1] {
                return false;
            }
        }
        true
    }

    /// Alias for is_monotonic_increasing.
    #[must_use]
    pub fn is_monotonic(&self) -> bool {
        self.is_monotonic_increasing()
    }

    #[must_use]
    pub fn is_monotonic_decreasing(&self) -> bool {
        if self.labels.len() <= 1 {
            return true;
        }
        for pair in self.labels.windows(2) {
            if pair[0] < pair[1] {
                return false;
            }
        }
        true
    }

    #[must_use]
    pub fn unique(&self) -> Self {
        // A strictly-ascending index (every recognized SortOrder) is already
        // all-unique in first-seen order, so unique() is an identity — return an
        // O(1) Arc-sharing clone instead of hashing every label and rebuilding
        // the vector (br-frankenpandas-idxdup dedup family).
        if !matches!(self.sort_order(), SortOrder::Unsorted) {
            return self.clone();
        }
        let mut seen = FxHashMap::<&IndexLabel, ()>::default();
        let labels: Vec<IndexLabel> = self
            .labels
            .iter()
            .filter(|l| seen.insert(l, ()).is_none())
            .cloned()
            .collect();
        self.propagate_name(Self::new(labels))
    }

    #[must_use]
    pub fn duplicated(&self, keep: DuplicateKeep) -> Vec<bool> {
        let mut result = vec![false; self.labels.len()];
        // Strictly-ascending => no duplicates under any keep mode; skip hashing.
        if !matches!(self.sort_order(), SortOrder::Unsorted) {
            return result;
        }
        match keep {
            DuplicateKeep::First => {
                let mut seen = FxHashMap::<&IndexLabel, ()>::default();
                for (i, label) in self.labels.iter().enumerate() {
                    if seen.insert(label, ()).is_some() {
                        result[i] = true;
                    }
                }
            }
            DuplicateKeep::Last => {
                let mut seen = FxHashMap::<&IndexLabel, ()>::default();
                for (i, label) in self.labels.iter().enumerate().rev() {
                    if seen.insert(label, ()).is_some() {
                        result[i] = true;
                    }
                }
            }
            DuplicateKeep::None => {
                let mut counts = FxHashMap::<&IndexLabel, usize>::default();
                for label in &self.labels {
                    *counts.entry(label).or_insert(0) += 1;
                }
                for (i, label) in self.labels.iter().enumerate() {
                    if counts[label] > 1 {
                        result[i] = true;
                    }
                }
            }
        }
        result
    }

    #[must_use]
    pub fn drop_duplicates(&self) -> Self {
        self.drop_duplicates_keep(DuplicateKeep::First)
    }

    /// Drop duplicated labels with explicit keep behavior.
    ///
    /// Matches `pd.Index.drop_duplicates(keep=...)`.
    #[must_use]
    pub fn drop_duplicates_keep(&self, keep: DuplicateKeep) -> Self {
        // Strictly-ascending => nothing is dropped; O(1) Arc-sharing clone.
        if !matches!(self.sort_order(), SortOrder::Unsorted) {
            return self.clone();
        }
        let duplicated = self.duplicated(keep);
        let labels = self
            .labels
            .iter()
            .zip(duplicated)
            .filter(|(_, is_duplicated)| !is_duplicated)
            .map(|(label, _)| label.clone())
            .collect();
        self.propagate_name(Self::new(labels))
    }

    // ── Pandas Index Model: set operations ─────────────────────────────

    #[must_use]
    pub fn intersection(&self, other: &Self) -> Self {
        // Both strictly ascending (every SortOrder::Ascending* is globally
        // IndexLabel::Ord-sorted and unique) => a two-pointer merge yields the
        // same self-ordered, deduplicated intersection without building either
        // FxHashMap (br-frankenpandas-idxdup set ops).
        if let Some(labels) = self.sorted_merge_set_op(other, SetMergeKind::Intersection) {
            let mut result = Self::new(labels);
            result.name = self.shared_name(other);
            return result;
        }
        let other_set = other.position_map_first_ref();
        let mut seen = FxHashMap::<&IndexLabel, ()>::default();
        let labels: Vec<IndexLabel> = self
            .labels
            .iter()
            .filter(|l| other_set.contains_key(l) && seen.insert(l, ()).is_none())
            .cloned()
            .collect();
        let mut result = Self::new(labels);
        result.name = self.shared_name(other);
        result
    }

    /// Hash-free two-pointer set merge for two strictly-ascending (hence
    /// `IndexLabel::Ord`-sorted and unique) indexes; returns `None` when either
    /// side is unsorted so the caller keeps its FxHashMap path. Emits labels in
    /// `self`'s order, which equals the sorted order on the fast path — exactly
    /// what the hash path's `self`-iteration-order filter produces.
    fn sorted_merge_set_op(&self, other: &Self, kind: SetMergeKind) -> Option<Vec<IndexLabel>> {
        if matches!(self.sort_order(), SortOrder::Unsorted)
            || matches!(other.sort_order(), SortOrder::Unsorted)
        {
            return None;
        }
        let a = self.labels();
        let b = other.labels();
        let mut labels = Vec::with_capacity(a.len().min(b.len()));
        let (mut i, mut j) = (0usize, 0usize);
        while i < a.len() {
            if j >= b.len() {
                if kind == SetMergeKind::Difference {
                    labels.extend_from_slice(&a[i..]);
                }
                break;
            }
            match a[i].cmp(&b[j]) {
                std::cmp::Ordering::Less => {
                    if kind == SetMergeKind::Difference {
                        labels.push(a[i].clone());
                    }
                    i += 1;
                }
                std::cmp::Ordering::Greater => j += 1,
                std::cmp::Ordering::Equal => {
                    if kind == SetMergeKind::Intersection {
                        labels.push(a[i].clone());
                    }
                    i += 1;
                    j += 1;
                }
            }
        }
        Some(labels)
    }

    #[must_use]
    pub fn union_with(&self, other: &Self) -> Self {
        let mut seen = FxHashMap::<&IndexLabel, ()>::default();
        let mut labels = Vec::with_capacity(self.labels.len() + other.labels.len());
        for label in self.labels.iter().chain(other.labels.iter()) {
            if seen.insert(label, ()).is_none() {
                labels.push(label.clone());
            }
        }
        let mut result = Self::new(labels);
        result.name = self.shared_name(other);
        result
    }

    #[must_use]
    pub fn difference(&self, other: &Self) -> Self {
        // Two-pointer merge when both sides are strictly ascending (see
        // intersection / sorted_merge_set_op).
        if let Some(labels) = self.sorted_merge_set_op(other, SetMergeKind::Difference) {
            return self.propagate_name(Self::new(labels));
        }
        let other_set = other.position_map_first_ref();
        let mut seen = FxHashMap::<&IndexLabel, ()>::default();
        let labels: Vec<IndexLabel> = self
            .labels
            .iter()
            .filter(|l| !other_set.contains_key(l) && seen.insert(l, ()).is_none())
            .cloned()
            .collect();
        self.propagate_name(Self::new(labels))
    }

    #[must_use]
    pub fn symmetric_difference(&self, other: &Self) -> Self {
        let self_set = self.position_map_first_ref();
        let other_set = other.position_map_first_ref();
        let mut seen = FxHashMap::<&IndexLabel, ()>::default();
        let mut labels = Vec::new();
        for label in &self.labels {
            if !other_set.contains_key(label) && seen.insert(label, ()).is_none() {
                labels.push(label.clone());
            }
        }
        for label in &other.labels {
            if !self_set.contains_key(label) && seen.insert(label, ()).is_none() {
                labels.push(label.clone());
            }
        }
        let mut result = Self::new(labels);
        result.name = self.shared_name(other);
        result
    }

    // ── Pandas Index Model: ordering and slicing ───────────────────────

    #[must_use]
    pub fn argsort(&self) -> Vec<usize> {
        let mut indices: Vec<usize> = (0..self.labels.len()).collect();
        indices.sort_by(|&a, &b| self.labels[a].cmp(&self.labels[b]));
        indices
    }

    #[must_use]
    pub fn sort_values(&self) -> Self {
        let order = self.argsort();
        self.propagate_name(Self::new(
            order.iter().map(|&i| self.labels[i].clone()).collect(),
        ))
    }

    #[must_use]
    pub fn take(&self, indices: &[usize]) -> Self {
        self.propagate_name(Self::new(
            indices.iter().map(|&i| self.labels[i].clone()).collect(),
        ))
    }

    #[must_use]
    pub fn slice(&self, start: usize, len: usize) -> Self {
        self.propagate_name(Self {
            labels: self.labels.slice(start, len),
            name: None,
            label_identity: next_index_label_identity(),
            duplicate_cache: OnceLock::new(),
            sort_order_cache: OnceLock::new(),
            semantic_fingerprint_cache: OnceLock::new(),
        })
    }

    #[must_use]
    pub fn from_range(start: i64, stop: i64, step: i64) -> Self {
        let mut labels = Vec::new();
        let mut val = start;
        if step > 0 {
            while val < stop {
                labels.push(IndexLabel::Int64(val));
                val += step;
            }
        } else if step < 0 {
            while val > stop {
                labels.push(IndexLabel::Int64(val));
                val += step;
            }
        }
        Self::new(labels)
    }

    // ── Pandas Index Model: aggregation ──────────────────────────────

    /// Minimum label.
    ///
    /// Matches `pd.Index.min()`.
    #[must_use]
    pub fn min(&self) -> Option<&IndexLabel> {
        self.labels.iter().min()
    }

    /// Maximum label.
    ///
    /// Matches `pd.Index.max()`.
    #[must_use]
    pub fn max(&self) -> Option<&IndexLabel> {
        self.labels.iter().max()
    }

    /// Position of the minimum label.
    ///
    /// Matches `pd.Index.argmin()`.
    #[must_use]
    pub fn argmin(&self) -> Option<usize> {
        self.labels
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| a.cmp(b))
            .map(|(i, _)| i)
    }

    /// Position of the maximum label.
    ///
    /// Matches `pd.Index.argmax()`.
    #[must_use]
    pub fn argmax(&self) -> Option<usize> {
        self.labels
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.cmp(b))
            .map(|(i, _)| i)
    }

    /// Number of unique labels.
    ///
    /// Matches `pd.Index.nunique()`.
    #[must_use]
    pub fn nunique(&self) -> usize {
        self.nunique_with_dropna(true)
    }

    /// Number of unique labels with explicit missing-value control.
    ///
    /// Matches `pd.Index.nunique(dropna=...)`.
    #[must_use]
    pub fn nunique_with_dropna(&self, dropna: bool) -> usize {
        self.unique()
            .labels
            .iter()
            .filter(|label| !dropna || !label.is_missing())
            .count()
    }

    // ── Pandas Index Model: transformation ───────────────────────────

    /// Apply a function to each label, producing a new Index.
    ///
    /// Matches `pd.Index.map(func)`.
    #[must_use]
    pub fn map<F>(&self, func: F) -> Self
    where
        F: Fn(&IndexLabel) -> IndexLabel,
    {
        self.propagate_name(Self::new(self.labels.iter().map(&func).collect()))
    }

    /// Rename the index (create a copy with transformed labels).
    ///
    /// Matches `pd.Index.rename(name)` / `pd.Index.set_names(name)`.
    /// Applies a function to each label.
    #[must_use]
    pub fn rename<F>(&self, func: F) -> Self
    where
        F: Fn(&IndexLabel) -> IndexLabel,
    {
        self.map(func)
    }

    /// Drop specific labels from the index.
    ///
    /// Matches `pd.Index.drop(labels)`.
    #[must_use]
    pub fn drop_labels(&self, labels_to_drop: &[IndexLabel]) -> Self {
        self.propagate_name(Self::new(
            self.labels
                .iter()
                .filter(|l| !labels_to_drop.contains(l))
                .cloned()
                .collect(),
        ))
    }

    /// Convert all labels to Int64 (if possible) or Utf8.
    ///
    /// Matches `pd.Index.astype(dtype)`. Returns a new Index with labels
    /// converted to the target type representation.
    #[must_use]
    pub fn astype_int(&self) -> Self {
        self.propagate_name(Self::new(
            self.labels
                .iter()
                .map(|l| match l {
                    IndexLabel::Int64(_) => l.clone(),
                    IndexLabel::Float64(v) => IndexLabel::Int64(v.0 as i64),
                    IndexLabel::Bool(b) => IndexLabel::Int64(i64::from(*b)),
                    IndexLabel::Utf8(s) => s
                        .parse::<i64>()
                        .map_or_else(|_| l.clone(), IndexLabel::Int64),
                    IndexLabel::Timedelta64(ns) => IndexLabel::Int64(*ns),
                    IndexLabel::Datetime64(ns) => IndexLabel::Int64(*ns),
                    // Missing labels have no integer form; preserved like
                    // unparseable strings (pandas astype(int) raises on NaN —
                    // callers reject before reaching here).
                    IndexLabel::Null(_) => l.clone(),
                })
                .collect(),
        ))
    }

    /// Convert all labels to Utf8 strings.
    ///
    /// Matches `pd.Index.astype(str)`.
    #[must_use]
    pub fn astype_str(&self) -> Self {
        self.propagate_name(Self::new(
            self.labels
                .iter()
                .map(|l| match l {
                    IndexLabel::Int64(v) => IndexLabel::Utf8(v.to_string()),
                    IndexLabel::Float64(v) => IndexLabel::Utf8(if v.0.is_nan() {
                        "nan".to_owned()
                    } else {
                        let t = format!("{}", v.0);
                        if t.contains('.') || t.contains('e') || t.contains("inf") {
                            t
                        } else {
                            format!("{t}.0")
                        }
                    }),
                    IndexLabel::Bool(b) => {
                        IndexLabel::Utf8(if *b { "True" } else { "False" }.to_owned())
                    }
                    IndexLabel::Utf8(_) => l.clone(),
                    IndexLabel::Timedelta64(ns) => IndexLabel::Utf8(Timedelta::format(*ns)),
                    IndexLabel::Datetime64(ns) => IndexLabel::Utf8(format_datetime_ns(*ns)),
                    // astype(str) uses Python str() forms: str(None)=='None',
                    // str(nan)=='nan' (LOWERCASE, unlike the repr surface),
                    // str(NaT)=='NaT'. Verified pandas 2.2.3.
                    IndexLabel::Null(kind) => IndexLabel::Utf8(
                        match kind {
                            fp_types::NullKind::Null => "None",
                            fp_types::NullKind::NaN => "nan",
                            fp_types::NullKind::NaT => "NaT",
                        }
                        .to_owned(),
                    ),
                })
                .collect(),
        ))
    }

    /// Convert labels to a pandas dtype string.
    ///
    /// Matches `pd.Index.astype(dtype)` for the generic dtype names this crate
    /// can represent directly.
    pub fn astype(&self, dtype: &str) -> Result<Self, IndexError> {
        match dtype {
            "int" | "int64" => Ok(self.astype_int()),
            "str" | "string" | "object" => Ok(self.astype_str()),
            "datetime64[ns]" => {
                ensure_index_kind(
                    self,
                    |label| matches!(label, IndexLabel::Datetime64(_)),
                    "DatetimeIndex",
                )?;
                Ok(self.clone())
            }
            "timedelta64[ns]" => {
                ensure_index_kind(
                    self,
                    |label| matches!(label, IndexLabel::Timedelta64(_)),
                    "TimedeltaIndex",
                )?;
                Ok(self.clone())
            }
            other => Err(IndexError::InvalidArgument(format!(
                "unsupported Index.astype dtype {other:?}"
            ))),
        }
    }

    /// Equality check against another Index.
    ///
    /// Matches `pd.Index.equals(other)`. Returns true iff `other` has
    /// the same labels in the same order. Names are ignored (use
    /// `identical` for a name-sensitive check).
    #[must_use]
    pub fn equals(&self, other: &Self) -> bool {
        self.labels_equal(other)
    }

    /// Strict equality including name.
    ///
    /// Matches `pd.Index.identical(other)`. Requires the same labels in
    /// the same order AND the same name.
    #[must_use]
    pub fn identical(&self, other: &Self) -> bool {
        self.labels_equal(other) && self.name == other.name
    }

    fn value_counts_raw(
        &self,
        sort: bool,
        ascending: bool,
        dropna: bool,
    ) -> (Vec<(IndexLabel, usize)>, usize) {
        let mut seen_order: Vec<IndexLabel> = Vec::new();
        let mut counts: FxHashMap<IndexLabel, usize> = FxHashMap::default();
        let mut total = 0usize;
        for label in &self.labels {
            if dropna && label.is_missing() {
                continue;
            }
            total += 1;
            if !counts.contains_key(label) {
                seen_order.push(label.clone());
            }
            *counts.entry(label.clone()).or_insert(0) += 1;
        }
        let mut pairs: Vec<(IndexLabel, usize)> = seen_order
            .into_iter()
            .map(|label| {
                let count = counts[&label];
                (label, count)
            })
            .collect();
        if sort {
            if ascending {
                pairs.sort_by_key(|entry| entry.1);
            } else {
                pairs.sort_by_key(|entry| std::cmp::Reverse(entry.1));
            }
        }
        (pairs, total)
    }

    /// Count occurrences of each distinct label.
    ///
    /// Matches `pd.Index.value_counts()` default behavior. Missing labels are
    /// dropped, counts are sorted descending, and first-seen order breaks ties.
    #[must_use]
    pub fn value_counts(&self) -> Vec<(IndexLabel, usize)> {
        self.value_counts_raw(true, false, true).0
    }

    /// Count occurrences of each distinct label with pandas-style options.
    ///
    /// Matches `pd.Index.value_counts(normalize, sort, ascending, dropna)`.
    /// Returns `Scalar::Int64` counts unless `normalize=true`, in which case
    /// the values are `Scalar::Float64` fractions.
    #[must_use]
    pub fn value_counts_with_options(
        &self,
        normalize: bool,
        sort: bool,
        ascending: bool,
        dropna: bool,
    ) -> Vec<(IndexLabel, Scalar)> {
        let (pairs, total) = self.value_counts_raw(sort, ascending, dropna);
        if normalize {
            let denom = total as f64;
            return pairs
                .into_iter()
                .map(|(label, count)| (label, Scalar::Float64(count as f64 / denom)))
                .collect();
        }

        pairs
            .into_iter()
            .map(|(label, count)| (label, Scalar::Int64(count as i64)))
            .collect()
    }

    /// Shift the labels by `periods` positions, filling vacated slots
    /// with `fill`.
    ///
    /// Matches `pd.Index.shift(periods, fill_value=...)` for the
    /// positional form (pandas also supports a `freq`-aware shift for
    /// datetime indexes; that path is out of scope here). Positive
    /// periods shift right; negative shift left.
    #[must_use]
    pub fn shift(&self, periods: i64, fill: IndexLabel) -> Self {
        let len = self.labels.len();
        if len == 0 || periods == 0 {
            return self.clone();
        }
        let mut out: Vec<IndexLabel> = Vec::with_capacity(len);
        let abs = periods.unsigned_abs() as usize;
        if abs >= len {
            for _ in 0..len {
                out.push(fill.clone());
            }
        } else if periods > 0 {
            for _ in 0..abs {
                out.push(fill.clone());
            }
            out.extend_from_slice(&self.labels[..len - abs]);
        } else {
            out.extend_from_slice(&self.labels[abs..]);
            for _ in 0..abs {
                out.push(fill.clone());
            }
        }
        self.propagate_name(Self::new(out))
    }

    /// Nearest-preceding-or-equal label lookup.
    ///
    /// Matches `pd.Index.asof(label)` for monotonic-increasing
    /// indexes: returns the largest label `<= key`. Returns `None`
    /// when no such label exists (key precedes every entry). The
    /// index is assumed sorted; callers should `sort_values()` first
    /// if needed (pandas emits a warning in the non-monotonic case
    /// but still does a linear scan — we match that behavior).
    #[must_use]
    pub fn asof(&self, key: &IndexLabel) -> Option<IndexLabel> {
        let mut best: Option<&IndexLabel> = None;
        for label in &self.labels {
            if label.is_missing() {
                continue;
            }
            if label.cmp(key).is_le() {
                best = Some(label);
            } else {
                break;
            }
        }
        best.cloned()
    }

    /// Position where `value` would be inserted to keep the index
    /// sorted ascending.
    ///
    /// Matches `pd.Index.searchsorted(value, side)`. `side` is
    /// `"left"` (first valid insertion) or `"right"` (last). Returns
    /// an error for unknown sides or missing needles.
    pub fn searchsorted(&self, value: &IndexLabel, side: &str) -> Result<usize, IndexError> {
        if side != "left" && side != "right" {
            return Err(IndexError::InvalidArgument(format!(
                "searchsorted: side must be 'left' or 'right', got {side:?}"
            )));
        }
        if value.is_missing() {
            return Err(IndexError::InvalidArgument(
                "searchsorted: needle cannot be missing".to_owned(),
            ));
        }
        let mut lo = 0usize;
        let mut hi = self.labels.len();
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let cmp = if self.labels[mid].is_missing() {
                std::cmp::Ordering::Greater
            } else {
                self.labels[mid].cmp(value)
            };
            use std::cmp::Ordering;
            let go_right = matches!(
                (cmp, side),
                (Ordering::Less, _) | (Ordering::Equal, "right")
            );
            if go_right {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        Ok(lo)
    }

    /// Approximate memory footprint (bytes) occupied by the labels.
    ///
    /// Matches `pd.Index.memory_usage(deep=...)`. `deep=false` uses
    /// a fixed per-label width (8 bytes for Int64/Timedelta64/
    /// Datetime64, pointer-size for Utf8); `deep=true` additionally
    /// accounts for each Utf8 string's byte length.
    #[must_use]
    pub fn memory_usage(&self, deep: bool) -> usize {
        self.labels
            .iter()
            .map(|label| match label {
                IndexLabel::Int64(_)
                | IndexLabel::Float64(_)
                | IndexLabel::Timedelta64(_)
                | IndexLabel::Datetime64(_)
                | IndexLabel::Null(_) => 8,
                IndexLabel::Bool(_) => 1,
                IndexLabel::Utf8(s) => {
                    if deep {
                        std::mem::size_of::<String>() + s.len()
                    } else {
                        std::mem::size_of::<String>()
                    }
                }
            })
            .sum()
    }

    /// Number of levels in this index.
    ///
    /// Matches `pd.Index.nlevels`. Always 1 for the flat Index type;
    /// MultiIndex already overrides this. Provided so callers can
    /// write level-agnostic code that works on either kind.
    #[must_use]
    pub fn nlevels(&self) -> usize {
        1
    }

    /// Materialize labels into an owned `Vec<IndexLabel>`.
    ///
    /// Matches `pd.Index.to_list()`. Convenience helper for callers
    /// that need ownership without manually cloning via `labels()`.
    #[must_use]
    pub fn to_list(&self) -> Vec<IndexLabel> {
        self.labels().to_vec()
    }

    /// Stringify each label using its `Display` impl.
    ///
    /// Matches `pd.Index.format()` / `pd.Index.astype(str).tolist()`.
    /// Result is a `Vec<String>` in index order.
    #[must_use]
    pub fn format(&self) -> Vec<String> {
        self.labels.iter().map(IndexLabel::to_string).collect()
    }

    /// Replace labels at positions where `cond` is true with `value`.
    ///
    /// Matches `pd.Index.putmask(cond, value)`. A shorter `cond`
    /// leaves trailing labels unchanged (pandas-style lenient
    /// alignment); a longer `cond` is silently truncated. The name
    /// is preserved.
    #[must_use]
    pub fn putmask(&self, cond: &[bool], value: &IndexLabel) -> Self {
        let new_labels: Vec<IndexLabel> = self
            .labels
            .iter()
            .enumerate()
            .map(|(i, label)| {
                if cond.get(i).copied().unwrap_or(false) {
                    value.clone()
                } else {
                    label.clone()
                }
            })
            .collect();
        self.propagate_name(Self::new(new_labels))
    }

    /// Whether any label coerces to true.
    ///
    /// Matches `pd.Index.any()`. Non-zero integers, non-empty strings,
    /// and non-NaT timedeltas count as truthy. Missing labels are
    /// treated as falsy. Empty index returns false.
    #[must_use]
    pub fn any(&self) -> bool {
        self.labels.iter().any(index_label_is_truthy)
    }

    /// Whether all labels coerce to true.
    ///
    /// Matches `pd.Index.all()`. Empty index returns true (pandas
    /// convention: vacuously true). Missing labels count as falsy.
    #[must_use]
    pub fn all(&self) -> bool {
        self.labels.iter().all(index_label_is_truthy)
    }

    /// Drop missing labels, preserving order.
    ///
    /// Matches `pd.Index.dropna()`. Labels whose `is_missing()` returns
    /// true are removed. The name (if any) is preserved.
    #[must_use]
    pub fn dropna(&self) -> Self {
        self.propagate_name(Self::new(
            self.labels
                .iter()
                .filter(|label| !label.is_missing())
                .cloned()
                .collect(),
        ))
    }

    /// Insert a new label at the given position.
    ///
    /// Matches `pd.Index.insert(loc, item)`. `loc` is an ordinal position
    /// where the new label is inserted; positions equal to `len()` append
    /// to the end. Out-of-bounds positions return an `OutOfBounds` error.
    pub fn insert(&self, loc: usize, item: IndexLabel) -> Result<Self, IndexError> {
        if loc > self.labels.len() {
            return Err(IndexError::OutOfBounds {
                position: loc,
                length: self.labels.len(),
            });
        }
        let mut labels = self.labels().to_vec();
        labels.insert(loc, item);
        Ok(self.propagate_name(Self::new(labels)))
    }

    /// Delete the label at the given position.
    ///
    /// Matches `pd.Index.delete(loc)`. Returns an `OutOfBounds` error
    /// for positions outside `0..len()`.
    pub fn delete(&self, loc: usize) -> Result<Self, IndexError> {
        if loc >= self.labels.len() {
            return Err(IndexError::OutOfBounds {
                position: loc,
                length: self.labels.len(),
            });
        }
        let mut labels = self.labels().to_vec();
        labels.remove(loc);
        Ok(self.propagate_name(Self::new(labels)))
    }

    /// Append another index to the end of this one.
    ///
    /// Matches `pd.Index.append(other)`. The returned index contains
    /// `self.labels` followed by `other.labels`. Name is preserved from
    /// `self`.
    #[must_use]
    pub fn append(&self, other: &Self) -> Self {
        let mut labels = self.labels().to_vec();
        labels.extend(other.labels.iter().cloned());
        self.propagate_name(Self::new(labels))
    }

    /// Repeat each label `repeats` times.
    ///
    /// Matches `pd.Index.repeat(repeats)`. `repeats=0` yields an empty
    /// index; `repeats=1` is a no-op clone. Name is preserved.
    #[must_use]
    pub fn repeat(&self, repeats: usize) -> Self {
        if repeats == 0 {
            return self.propagate_name(Self::new(Vec::new()));
        }
        if repeats == 1 {
            return self.clone();
        }
        let mut out = Vec::with_capacity(self.labels.len() * repeats);
        for label in &self.labels {
            for _ in 0..repeats {
                out.push(label.clone());
            }
        }
        self.propagate_name(Self::new(out))
    }

    /// Fill missing labels with the provided scalar.
    ///
    /// Matches `pd.Index.fillna(value)`.
    #[must_use]
    pub fn fillna(&self, value: &IndexLabel) -> Self {
        self.propagate_name(Self::new(
            self.labels
                .iter()
                .map(|label| {
                    if label.is_missing() {
                        value.clone()
                    } else {
                        label.clone()
                    }
                })
                .collect(),
        ))
    }

    /// Matches `pd.Index.isna()`.
    #[must_use]
    pub fn isna(&self) -> Vec<bool> {
        self.labels.iter().map(IndexLabel::is_missing).collect()
    }

    /// Matches `pd.Index.notna()`.
    #[must_use]
    pub fn notna(&self) -> Vec<bool> {
        self.labels
            .iter()
            .map(|label| !label.is_missing())
            .collect()
    }

    /// Where: replace labels at false positions with a fill value.
    ///
    /// Matches `pd.Index.where(cond, other)`.
    #[must_use]
    pub fn where_cond(&self, cond: &[bool], other: &IndexLabel) -> Self {
        self.propagate_name(Self::new(
            self.labels
                .iter()
                .enumerate()
                .map(|(i, l)| {
                    if cond.get(i).copied().unwrap_or(false) {
                        l.clone()
                    } else {
                        other.clone()
                    }
                })
                .collect(),
        ))
    }

    /// Alias for `union_with`, matching `pd.Index.union`.
    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        self.union_with(other)
    }

    /// Alias for `sort_values`, matching `pd.Index.sort`.
    #[must_use]
    pub fn sort(&self) -> Self {
        self.sort_values()
    }

    /// Sort labels and return the positional indexer used for the sort.
    ///
    /// Matches the flat-index shape of `pd.Index.sortlevel()`.
    #[must_use]
    pub fn sortlevel(&self) -> (Self, Vec<usize>) {
        let order = self.argsort();
        (self.take(&order), order)
    }

    /// Alias for `drop_labels`, matching `pd.Index.drop`.
    #[must_use]
    pub fn drop(&self, labels_to_drop: &[IndexLabel]) -> Self {
        self.drop_labels(labels_to_drop)
    }

    /// Clone this index, matching `pd.Index.copy`.
    #[must_use]
    pub fn copy(&self) -> Self {
        self.clone()
    }

    /// Alias for `where_cond`, matching `pd.Index.where`.
    #[must_use]
    pub fn where_(&self, cond: &[bool], other: &IndexLabel) -> Self {
        self.where_cond(cond, other)
    }

    /// Alias for `to_list`, matching `pd.Index.tolist`.
    #[must_use]
    pub fn tolist(&self) -> Vec<IndexLabel> {
        self.to_list()
    }

    /// Object-array-shaped materialization, matching `pd.Index.to_numpy`.
    #[must_use]
    pub fn to_numpy(&self) -> Vec<IndexLabel> {
        self.to_list()
    }

    /// Alias for `to_numpy`, matching `pd.Index.array`.
    #[must_use]
    pub fn array(&self) -> Vec<IndexLabel> {
        self.to_numpy()
    }

    /// Alias for `to_numpy`, matching `pd.Index.values`.
    #[must_use]
    pub fn values(&self) -> Vec<IndexLabel> {
        self.to_numpy()
    }

    /// Alias for `to_numpy`, matching `pd.Index.ravel`.
    #[must_use]
    pub fn ravel(&self) -> Vec<IndexLabel> {
        self.to_numpy()
    }

    /// Return a shallow clone view, matching `pd.Index.view` for this
    /// immutable Rust representation.
    #[must_use]
    pub fn view(&self) -> Self {
        self.clone()
    }

    /// Flat-index transpose is identity, matching `pd.Index.transpose`.
    #[must_use]
    pub fn transpose(&self) -> Self {
        self.clone()
    }

    /// Alias for `transpose`, matching `pd.Index.T`.
    #[allow(non_snake_case)]
    #[must_use]
    pub fn T(&self) -> Self {
        self.transpose()
    }

    /// One-column row materialization, matching the flat-index shape of
    /// `pd.Index.to_frame(index=False)`.
    #[must_use]
    pub fn to_frame(&self) -> Vec<Vec<IndexLabel>> {
        self.labels
            .iter()
            .map(|label| vec![label.clone()])
            .collect()
    }

    /// Series-shaped materialization using the index labels as both index and
    /// values until `fp-frame` owns the richer return type.
    #[must_use]
    pub fn to_series(&self) -> Vec<(IndexLabel, IndexLabel)> {
        self.labels
            .iter()
            .map(|label| (label.clone(), label.clone()))
            .collect()
    }

    /// Pandas dtype string for this flat index.
    #[must_use]
    pub fn dtype(&self) -> &'static str {
        match self.inferred_type() {
            "integer" => "int64",
            "string" => "object",
            "timedelta64" => "timedelta64[ns]",
            "datetime64" => "datetime64[ns]",
            "empty" | "mixed" => "object",
            _ => "object",
        }
    }

    /// One-element dtype list, matching the `.dtypes` accessor shape used by
    /// pandas containers.
    #[must_use]
    pub fn dtypes(&self) -> Vec<&'static str> {
        vec![self.dtype()]
    }

    /// Infer object labels without changing the current typed representation.
    #[must_use]
    pub fn infer_objects(&self) -> Self {
        self.clone()
    }

    /// Whether this index's dtype can hold integer labels.
    #[must_use]
    pub fn holds_integer(&self) -> bool {
        self.is_integer()
    }

    /// Pandas-style inferred-type string for the label values.
    #[must_use]
    pub fn inferred_type(&self) -> &'static str {
        if self.labels.is_empty() {
            return "empty";
        }
        let mut non_missing = self.labels.iter().filter(|label| !label.is_missing());
        let Some(first) = non_missing.next() else {
            return "empty";
        };
        let same_kind = |label: &IndexLabel| {
            matches!(
                (first, label),
                (IndexLabel::Int64(_), IndexLabel::Int64(_))
                    | (IndexLabel::Float64(_), IndexLabel::Float64(_))
                    | (IndexLabel::Bool(_), IndexLabel::Bool(_))
                    | (IndexLabel::Utf8(_), IndexLabel::Utf8(_))
                    | (IndexLabel::Timedelta64(_), IndexLabel::Timedelta64(_))
                    | (IndexLabel::Datetime64(_), IndexLabel::Datetime64(_))
            )
        };
        if !non_missing.all(same_kind) {
            return "mixed";
        }
        match first {
            IndexLabel::Int64(_) => "integer",
            IndexLabel::Float64(_) => "floating",
            IndexLabel::Bool(_) => "boolean",
            IndexLabel::Utf8(_) => "string",
            IndexLabel::Timedelta64(_) => "timedelta64",
            IndexLabel::Datetime64(_) => "datetime64",
            // Unreachable: `first` comes from the non-missing iterator and
            // every Null label is_missing.
            IndexLabel::Null(_) => "mixed",
        }
    }

    /// Whether this index contains missing labels, matching `pd.Index.hasnans`.
    #[must_use]
    pub fn hasnans(&self) -> bool {
        self.labels.iter().any(IndexLabel::is_missing)
    }

    /// Number of dimensions, matching `pd.Index.ndim`.
    #[must_use]
    pub fn ndim(&self) -> usize {
        1
    }

    /// One-dimensional shape, matching `pd.Index.shape`.
    #[must_use]
    pub fn shape(&self) -> (usize,) {
        (self.len(),)
    }

    /// Number of entries, matching `pd.Index.size`.
    #[must_use]
    pub fn size(&self) -> usize {
        self.len()
    }

    /// Shallow byte footprint, matching `pd.Index.nbytes`.
    #[must_use]
    pub fn nbytes(&self) -> usize {
        self.memory_usage(false)
    }

    /// Alias for `is_empty`, matching the pandas `.empty` property.
    #[must_use]
    pub fn empty(&self) -> bool {
        self.is_empty()
    }

    /// Return the single contained label.
    ///
    /// Matches `pd.Index.item()`, which rejects indexes with length other than
    /// one.
    pub fn item(&self) -> Result<IndexLabel, IndexError> {
        if self.len() == 1 {
            Ok(self.labels[0].clone())
        } else {
            Err(IndexError::InvalidArgument(format!(
                "item requires exactly one label, got {}",
                self.len()
            )))
        }
    }

    /// Identity check, matching `pd.Index.is_`.
    #[must_use]
    pub fn is_(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }

    /// Whether all non-missing labels are booleans.
    #[must_use]
    pub fn is_boolean(&self) -> bool {
        false
    }

    /// Whether this generic index is categorical.
    #[must_use]
    pub fn is_categorical(&self) -> bool {
        false
    }

    /// Whether all non-missing labels are floating-point.
    #[must_use]
    pub fn is_floating(&self) -> bool {
        false
    }

    /// Whether all non-missing labels are Int64 labels.
    #[must_use]
    pub fn is_integer(&self) -> bool {
        !self.labels.is_empty()
            && self
                .labels
                .iter()
                .filter(|label| !label.is_missing())
                .all(|label| matches!(label, IndexLabel::Int64(_)))
    }

    /// Whether this generic index is interval-typed.
    #[must_use]
    pub fn is_interval(&self) -> bool {
        false
    }

    /// Whether all non-missing labels are numeric.
    #[must_use]
    pub fn is_numeric(&self) -> bool {
        self.is_integer()
    }

    /// Whether this index is object-backed.
    #[must_use]
    pub fn is_object(&self) -> bool {
        matches!(self.dtype(), "object")
    }

    /// Alias for `isna`, matching `pd.Index.isnull`.
    #[must_use]
    pub fn isnull(&self) -> Vec<bool> {
        self.isna()
    }

    /// Alias for `notna`, matching `pd.Index.notnull`.
    #[must_use]
    pub fn notnull(&self) -> Vec<bool> {
        self.notna()
    }

    /// Factorize labels into integer codes and unique labels.
    ///
    /// Missing labels receive code `-1`; non-missing labels preserve first-seen
    /// order in the returned uniques index.
    #[must_use]
    pub fn factorize(&self) -> (Vec<isize>, Self) {
        let mut positions = FxHashMap::<IndexLabel, isize>::default();
        let mut uniques = Vec::<IndexLabel>::new();
        let mut codes = Vec::with_capacity(self.labels.len());
        for label in &self.labels {
            if label.is_missing() {
                codes.push(-1);
            } else if let Some(code) = positions.get(label) {
                codes.push(*code);
            } else {
                let code = isize::try_from(uniques.len()).unwrap_or(isize::MAX);
                positions.insert(label.clone(), code);
                uniques.push(label.clone());
                codes.push(code);
            }
        }
        (codes, self.propagate_name(Self::new(uniques)))
    }

    /// Alias for `get_indexer`, matching `pd.Index.get_indexer_for`.
    #[must_use]
    pub fn get_indexer_for(&self, target: &Self) -> Vec<Option<usize>> {
        self.get_indexer(target)
    }

    /// Expand duplicate matches while indexing a target index.
    ///
    /// Matches `pd.Index.get_indexer_non_unique(target)` shape: every matching
    /// source position is emitted for each target label, and missing target
    /// ordinal positions are returned separately.
    #[must_use]
    pub fn get_indexer_non_unique(&self, target: &Self) -> (Vec<isize>, Vec<usize>) {
        let mut positions = FxHashMap::<IndexLabel, Vec<usize>>::default();
        for (position, label) in self.labels.iter().enumerate() {
            positions.entry(label.clone()).or_default().push(position);
        }

        let mut indexer = Vec::new();
        let mut missing = Vec::new();
        for (target_position, label) in target.labels.iter().enumerate() {
            if let Some(source_positions) = positions.get(label) {
                indexer.extend(
                    source_positions
                        .iter()
                        .map(|position| isize::try_from(*position).unwrap_or(isize::MAX)),
                );
            } else {
                indexer.push(-1);
                missing.push(target_position);
            }
        }
        (indexer, missing)
    }

    /// Get labels for a level. Flat indexes only accept level 0.
    pub fn get_level_values(&self, level: usize) -> Result<Self, IndexError> {
        if level == 0 {
            Ok(self.clone())
        } else {
            Err(IndexError::OutOfBounds {
                position: level,
                length: 1,
            })
        }
    }

    /// Bound for a label slice, matching `pd.Index.get_slice_bound`.
    pub fn get_slice_bound(&self, label: &IndexLabel, side: &str) -> Result<usize, IndexError> {
        self.searchsorted(label, side)
    }

    /// Return `(start, stop)` bounds for a label slice. Stop is exclusive.
    pub fn slice_locs(
        &self,
        start: Option<&IndexLabel>,
        end: Option<&IndexLabel>,
    ) -> Result<(usize, usize), IndexError> {
        let start = match start {
            Some(label) => self.get_slice_bound(label, "left")?,
            None => 0,
        };
        let end = match end {
            Some(label) => self.get_slice_bound(label, "right")?,
            None => self.len(),
        };
        Ok(if end < start {
            (start, start)
        } else {
            (start, end)
        })
    }

    /// Alias for `slice_locs`, matching `pd.Index.slice_indexer`.
    pub fn slice_indexer(
        &self,
        start: Option<&IndexLabel>,
        end: Option<&IndexLabel>,
    ) -> Result<(usize, usize), IndexError> {
        self.slice_locs(start, end)
    }

    /// Reindex to a target index, returning the target and source positions.
    #[must_use]
    pub fn reindex(&self, target: &Self) -> (Self, Vec<Option<usize>>) {
        (target.clone(), self.get_indexer(target))
    }

    /// Flat-index `droplevel` is invalid because it would remove the only
    /// level.
    pub fn droplevel(&self, level: usize) -> Result<Self, IndexError> {
        if level == 0 {
            Err(IndexError::InvalidArgument(
                "cannot remove the only level from a flat Index".to_owned(),
            ))
        } else {
            Err(IndexError::OutOfBounds {
                position: level,
                length: 1,
            })
        }
    }

    /// Rounding is a no-op for current discrete flat index labels.
    #[must_use]
    pub fn round(&self) -> Self {
        self.clone()
    }

    /// String accessor for Utf8 labels, matching `pd.Index.str`.
    #[must_use]
    pub fn r#str(&self) -> IndexStringAccessor<'_> {
        IndexStringAccessor::borrowed(self)
    }

    /// Group label positions by label value, matching `pd.Index.groupby`.
    #[must_use]
    pub fn groupby(&self) -> HashMap<IndexLabel, Vec<usize>> {
        let mut groups = HashMap::<IndexLabel, Vec<usize>>::new();
        for (position, label) in self.labels.iter().enumerate() {
            groups.entry(label.clone()).or_default().push(position);
        }
        groups
    }

    /// Join two flat indexes using pandas-style join modes.
    pub fn join(&self, other: &Self, how: &str) -> Result<Self, IndexError> {
        match how {
            "left" => Ok(self.clone()),
            "right" => Ok(other.clone()),
            "inner" => Ok(self.intersection(other)),
            "outer" => Ok(self.union_with(other)),
            other => Err(IndexError::InvalidArgument(format!(
                "join: how must be 'left', 'right', 'inner', or 'outer', got {other:?}"
            ))),
        }
    }

    /// Locate nearest preceding-or-equal positions for each target label.
    ///
    /// Matches `pd.Index.asof_locs(where, mask)` for monotonic flat indexes.
    #[must_use]
    pub fn asof_locs(&self, where_index: &Self, mask: Option<&[bool]>) -> Vec<Option<usize>> {
        where_index
            .labels
            .iter()
            .map(|key| {
                let mut best = None;
                for (position, label) in self.labels.iter().enumerate() {
                    if mask
                        .and_then(|values| values.get(position))
                        .is_some_and(|include| !include)
                    {
                        continue;
                    }
                    if label.is_missing() {
                        continue;
                    }
                    if label.cmp(key).is_le() {
                        best = Some(position);
                    } else {
                        break;
                    }
                }
                best
            })
            .collect()
    }

    /// Positional first differences for comparable scalar index labels.
    ///
    /// Int64 and Timedelta64 labels produce same-kind differences. Datetime64
    /// labels produce Timedelta64 deltas. Unsupported label combinations and
    /// overflow return `None` for that position.
    #[must_use]
    pub fn diff(&self, periods: usize) -> Vec<Option<IndexLabel>> {
        let mut out = vec![None; self.len()];
        if periods == 0 {
            return out;
        }
        for (position, slot) in out.iter_mut().enumerate().skip(periods) {
            *slot = match (&self.labels[position], &self.labels[position - periods]) {
                (IndexLabel::Int64(current), IndexLabel::Int64(previous)) => {
                    current.checked_sub(*previous).map(IndexLabel::Int64)
                }
                (IndexLabel::Timedelta64(current), IndexLabel::Timedelta64(previous))
                    if *current != Timedelta::NAT && *previous != Timedelta::NAT =>
                {
                    current.checked_sub(*previous).map(IndexLabel::Timedelta64)
                }
                (IndexLabel::Datetime64(current), IndexLabel::Datetime64(previous))
                    if *current != i64::MIN && *previous != i64::MIN =>
                {
                    current.checked_sub(*previous).map(IndexLabel::Timedelta64)
                }
                _ => None,
            };
        }
        out
    }
}

#[derive(Debug, Clone)]
pub struct IndexStringAccessor<'a> {
    index: Cow<'a, Index>,
}

impl<'a> IndexStringAccessor<'a> {
    fn borrowed(index: &'a Index) -> Self {
        Self {
            index: Cow::Borrowed(index),
        }
    }

    fn owned(index: Index) -> Self {
        Self {
            index: Cow::Owned(index),
        }
    }

    fn map_utf8<T>(&self, func: impl Fn(&str) -> T) -> Vec<Option<T>> {
        self.index
            .labels()
            .iter()
            .map(|label| match label {
                IndexLabel::Utf8(value) => Some(func(value)),
                IndexLabel::Int64(_)
                | IndexLabel::Float64(_)
                | IndexLabel::Bool(_)
                | IndexLabel::Timedelta64(_)
                | IndexLabel::Datetime64(_)
                | IndexLabel::Null(_) => None,
            })
            .collect()
    }

    /// Lowercase string labels.
    #[must_use]
    pub fn lower(&self) -> Vec<Option<String>> {
        self.map_utf8(str::to_lowercase)
    }

    /// Uppercase string labels.
    #[must_use]
    pub fn upper(&self) -> Vec<Option<String>> {
        self.map_utf8(str::to_uppercase)
    }

    /// Substring membership for string labels.
    #[must_use]
    pub fn contains(&self, needle: &str) -> Vec<Option<bool>> {
        self.map_utf8(|value| value.contains(needle))
    }

    /// String length for string labels.
    #[must_use]
    pub fn len(&self) -> Vec<Option<usize>> {
        self.map_utf8(str::len)
    }

    /// String emptiness for string labels.
    #[must_use]
    pub fn is_empty(&self) -> Vec<Option<bool>> {
        self.map_utf8(str::is_empty)
    }
}

fn datetime_from_nanos(nanos: i64) -> Option<chrono::DateTime<chrono::Utc>> {
    if nanos == i64::MIN {
        return None;
    }
    let secs = nanos.div_euclid(1_000_000_000);
    let subsec_nanos = nanos.rem_euclid(1_000_000_000) as u32;
    chrono::DateTime::from_timestamp(secs, subsec_nanos)
}

fn datetime_to_period_error(message: impl Into<String>) -> IndexError {
    IndexError::InvalidArgument(format!(
        "DatetimeIndex to_period failed: {}",
        message.into()
    ))
}

fn date_to_weekly_period_ordinal(date: chrono::NaiveDate) -> Result<i64, IndexError> {
    let base = period_epoch_date(1969, 12, 22)?;
    Ok(date.signed_duration_since(base).num_days().div_euclid(7))
}

fn business_period_anchor_date(date: chrono::NaiveDate) -> Result<chrono::NaiveDate, IndexError> {
    match date.weekday().num_days_from_monday() {
        5 => period_add_days(date, 2),
        6 => period_add_days(date, 1),
        _ => Ok(date),
    }
}

fn date_to_business_period_ordinal(date: chrono::NaiveDate) -> Result<i64, IndexError> {
    let adjusted = business_period_anchor_date(date)?;
    let days = adjusted
        .signed_duration_since(period_epoch_date(1970, 1, 1)?)
        .num_days();
    let rem_ordinal = match days.rem_euclid(7) {
        0 => 0,
        1 => 1,
        4 => 2,
        5 => 3,
        6 => 4,
        _ => {
            return Err(datetime_to_period_error(
                "business period anchor did not land on a business day",
            ));
        }
    };
    days.div_euclid(7)
        .checked_mul(5)
        .and_then(|base| base.checked_add(rem_ordinal))
        .ok_or_else(|| datetime_to_period_error("business ordinal overflow"))
}

fn business_period_end_anchor_date(
    date: chrono::NaiveDate,
) -> Result<chrono::NaiveDate, IndexError> {
    match date.weekday().num_days_from_monday() {
        5 => period_add_days(date, -1),
        6 => period_add_days(date, -2),
        _ => Ok(date),
    }
}

fn datetime_period_ordinal(nanos: i64, freq: PeriodFreq) -> Result<i64, IndexError> {
    let dt = datetime_from_nanos(nanos).ok_or_else(|| {
        datetime_to_period_error(format!("invalid or NaT datetime nanos {nanos}"))
    })?;
    let date = dt.date_naive();
    let year_offset = i64::from(date.year()) - 1970;
    match freq {
        PeriodFreq::Annual => Ok(year_offset),
        PeriodFreq::Quarterly => year_offset
            .checked_mul(4)
            .and_then(|base| base.checked_add(i64::from((date.month() - 1) / 3)))
            .ok_or_else(|| datetime_to_period_error("quarterly ordinal overflow")),
        PeriodFreq::Monthly => year_offset
            .checked_mul(12)
            .and_then(|base| base.checked_add(i64::from(date.month() - 1)))
            .ok_or_else(|| datetime_to_period_error("monthly ordinal overflow")),
        PeriodFreq::Daily => {
            let epoch = chrono::NaiveDate::from_ymd_opt(1970, 1, 1)
                .ok_or_else(|| datetime_to_period_error("invalid epoch boundary"))?;
            Ok(date.signed_duration_since(epoch).num_days())
        }
        PeriodFreq::Hourly => Ok(nanos.div_euclid(Timedelta::NANOS_PER_HOUR)),
        PeriodFreq::Minutely => Ok(nanos.div_euclid(Timedelta::NANOS_PER_MIN)),
        PeriodFreq::Secondly => Ok(nanos.div_euclid(Timedelta::NANOS_PER_SEC)),
        PeriodFreq::Weekly => date_to_weekly_period_ordinal(date),
        PeriodFreq::Business => date_to_business_period_ordinal(date),
        _ => Err(datetime_to_period_error("unsupported period frequency")),
    }
}

fn datetime_period_ordinal_at_boundary(
    nanos: i64,
    freq: PeriodFreq,
    boundary: PeriodBoundary,
) -> Result<i64, IndexError> {
    if freq == PeriodFreq::Business && matches!(boundary, PeriodBoundary::End) {
        let dt = datetime_from_nanos(nanos).ok_or_else(|| {
            datetime_to_period_error(format!("invalid or NaT datetime nanos {nanos}"))
        })?;
        return date_to_business_period_ordinal(business_period_end_anchor_date(dt.date_naive())?);
    }
    datetime_period_ordinal(nanos, freq)
}

fn datetime_nanos_to_period(nanos: i64, freq: PeriodFreq) -> Result<Period, IndexError> {
    datetime_period_ordinal(nanos, freq).map(|ordinal| Period::new(ordinal, freq))
}

fn map_datetime_labels<T, F>(labels: &[IndexLabel], func: F) -> Vec<Option<T>>
where
    F: Fn(chrono::DateTime<chrono::Utc>) -> T,
{
    labels
        .iter()
        .map(|label| match label {
            IndexLabel::Datetime64(nanos) => datetime_from_nanos(*nanos).map(&func),
            IndexLabel::Int64(_)
            | IndexLabel::Float64(_)
            | IndexLabel::Bool(_)
            | IndexLabel::Utf8(_)
            | IndexLabel::Timedelta64(_)
            | IndexLabel::Null(_) => None,
        })
        .collect()
}

fn time_to_nanos(time: chrono::NaiveTime) -> i64 {
    use chrono::Timelike;
    i64::from(time.num_seconds_from_midnight()) * 1_000_000_000 + i64::from(time.nanosecond())
}

fn parse_time_of_day_nanos(time: &str, context: &str) -> Result<i64, IndexError> {
    let trimmed = time.trim();
    for format in ["%H:%M:%S%.f", "%H:%M:%S", "%H:%M"] {
        if let Ok(parsed) = chrono::NaiveTime::parse_from_str(trimmed, format) {
            return Ok(time_to_nanos(parsed));
        }
    }
    Err(IndexError::InvalidArgument(format!(
        "{context}: invalid time {time:?}; expected HH:MM, HH:MM:SS, or fractional seconds"
    )))
}

fn datetime_label_time_nanos(label: &IndexLabel) -> Option<i64> {
    match label {
        IndexLabel::Datetime64(nanos) => {
            datetime_from_nanos(*nanos).map(|dt| time_to_nanos(dt.time()))
        }
        IndexLabel::Int64(_)
        | IndexLabel::Float64(_)
        | IndexLabel::Bool(_)
        | IndexLabel::Utf8(_)
        | IndexLabel::Timedelta64(_)
        | IndexLabel::Null(_) => None,
    }
}

fn time_nanos_in_between(
    time: i64,
    start: i64,
    end: i64,
    include_start: bool,
    include_end: bool,
) -> bool {
    let after_start = if include_start {
        time >= start
    } else {
        time > start
    };
    let before_end = if include_end { time <= end } else { time < end };
    if start <= end {
        after_start && before_end
    } else {
        after_start || before_end
    }
}

fn map_timedelta_labels<T, F>(labels: &[IndexLabel], func: F) -> Vec<Option<T>>
where
    F: Fn(i64) -> T,
{
    labels
        .iter()
        .map(|label| match label {
            IndexLabel::Timedelta64(nanos) if *nanos != Timedelta::NAT => Some(func(*nanos)),
            IndexLabel::Int64(_)
            | IndexLabel::Float64(_)
            | IndexLabel::Bool(_)
            | IndexLabel::Utf8(_)
            | IndexLabel::Timedelta64(_)
            | IndexLabel::Datetime64(_)
            | IndexLabel::Null(_) => None,
        })
        .collect()
}

fn timedelta_components_for_index(nanos: i64) -> TimedeltaComponents {
    let days = nanos.div_euclid(Timedelta::NANOS_PER_DAY);
    let rem = nanos.rem_euclid(Timedelta::NANOS_PER_DAY);

    let hours = rem / Timedelta::NANOS_PER_HOUR;
    let rem = rem % Timedelta::NANOS_PER_HOUR;
    let minutes = rem / Timedelta::NANOS_PER_MIN;
    let rem = rem % Timedelta::NANOS_PER_MIN;
    let seconds = rem / Timedelta::NANOS_PER_SEC;
    let rem = rem % Timedelta::NANOS_PER_SEC;
    let milliseconds = rem / Timedelta::NANOS_PER_MILLI;
    let rem = rem % Timedelta::NANOS_PER_MILLI;
    let microseconds = rem / Timedelta::NANOS_PER_MICRO;
    let nanoseconds = rem % Timedelta::NANOS_PER_MICRO;

    TimedeltaComponents {
        days,
        hours,
        minutes,
        seconds,
        milliseconds,
        microseconds,
        nanoseconds,
    }
}

#[derive(Clone, Copy)]
enum TemporalRoundMode {
    Floor,
    Ceil,
    Round,
}

#[derive(Clone, Copy)]
enum PeriodBoundary {
    Start,
    End,
}

fn parse_fixed_temporal_freq(freq: &str, context: &str) -> Result<i64, IndexError> {
    let trimmed = freq.trim();
    let unit_nanos = Timedelta::unit_to_nanos(trimmed)
        .or_else(|| Timedelta::parse(trimmed).ok())
        .ok_or_else(|| {
            IndexError::InvalidArgument(format!("{context}: invalid frequency {freq:?}"))
        })?;
    if unit_nanos <= 0 {
        return Err(IndexError::InvalidArgument(format!(
            "{context}: frequency must be positive, got {freq:?}"
        )));
    }
    Ok(unit_nanos)
}

fn round_nanos_to_unit(nanos: i64, unit_nanos: i64, mode: TemporalRoundMode) -> i64 {
    match mode {
        TemporalRoundMode::Floor => nanos.div_euclid(unit_nanos).saturating_mul(unit_nanos),
        TemporalRoundMode::Ceil => {
            let rem = nanos.rem_euclid(unit_nanos);
            if rem == 0 {
                nanos
            } else {
                nanos.saturating_add(unit_nanos - rem)
            }
        }
        TemporalRoundMode::Round => {
            let floor = nanos.div_euclid(unit_nanos);
            let rem = nanos.rem_euclid(unit_nanos);
            if rem == 0 {
                return nanos;
            }
            let twice_rem = i128::from(rem) * 2;
            let unit = i128::from(unit_nanos);
            let chosen = if twice_rem < unit {
                floor
            } else if twice_rem > unit {
                floor.saturating_add(1)
            } else if floor % 2 == 0 {
                floor
            } else {
                floor.saturating_add(1)
            };
            chosen.saturating_mul(unit_nanos)
        }
    }
}

fn positional_diff<T>(
    len: usize,
    periods: i64,
    mut diff_at: impl FnMut(usize, usize) -> Option<T>,
) -> Vec<Option<T>> {
    let mut out = (0..len).map(|_| None).collect::<Vec<_>>();
    if periods == 0 {
        for (position, slot) in out.iter_mut().enumerate() {
            *slot = diff_at(position, position);
        }
        return out;
    }
    let Ok(offset) = usize::try_from(periods.unsigned_abs()) else {
        return out;
    };
    if offset >= len {
        return out;
    }
    if periods > 0 {
        for (position, slot) in out.iter_mut().enumerate().skip(offset) {
            *slot = diff_at(position, position - offset);
        }
    } else {
        for (position, slot) in out.iter_mut().enumerate().take(len - offset) {
            *slot = diff_at(position, position + offset);
        }
    }
    out
}

fn optional_diffs_to_timedelta_index(
    values: Vec<Option<i64>>,
    name: Option<&str>,
) -> TimedeltaIndex {
    let mut out = TimedeltaIndex::new(
        values
            .into_iter()
            .map(|value| value.unwrap_or(Timedelta::NAT))
            .collect(),
    );
    if let Some(name) = name {
        out = out.set_name(name);
    }
    out
}

fn period_timestamp_error(message: impl Into<String>) -> IndexError {
    IndexError::InvalidArgument(format!(
        "PeriodIndex timestamp conversion failed: {}",
        message.into()
    ))
}

fn period_date_error(err: DateRangeError) -> IndexError {
    period_timestamp_error(err.to_string())
}

fn period_date_to_nanos(date: chrono::NaiveDate) -> Result<i64, IndexError> {
    date_to_midnight_nanos(date).map_err(period_date_error)
}

fn period_checked_add_nanos(nanos: i64, delta: i64) -> Result<i64, IndexError> {
    nanos
        .checked_add(delta)
        .ok_or_else(|| period_timestamp_error("nanosecond timestamp overflow"))
}

fn period_month_start(month_ordinal: i64) -> Result<chrono::NaiveDate, IndexError> {
    let year = 1970_i64
        .checked_add(month_ordinal.div_euclid(12))
        .ok_or_else(|| period_timestamp_error("year overflow"))?;
    let year = i32::try_from(year).map_err(|_| period_timestamp_error("year out of range"))?;
    let month = u32::try_from(month_ordinal.rem_euclid(12) + 1)
        .map_err(|_| period_timestamp_error("month out of range"))?;
    chrono::NaiveDate::from_ymd_opt(year, month, 1)
        .ok_or_else(|| period_timestamp_error("invalid month boundary"))
}

fn period_epoch_date(year: i32, month: u32, day: u32) -> Result<chrono::NaiveDate, IndexError> {
    chrono::NaiveDate::from_ymd_opt(year, month, day)
        .ok_or_else(|| period_timestamp_error("invalid epoch boundary"))
}

fn period_add_days(date: chrono::NaiveDate, days: i64) -> Result<chrono::NaiveDate, IndexError> {
    let delta = chrono::Duration::try_days(days)
        .ok_or_else(|| period_timestamp_error("day offset overflow"))?;
    date.checked_add_signed(delta)
        .ok_or_else(|| period_timestamp_error("date overflow"))
}

fn period_business_date(ordinal: i64) -> Result<chrono::NaiveDate, IndexError> {
    let week = ordinal.div_euclid(5);
    let day_offset = match ordinal.rem_euclid(5) {
        0 => 0,
        1 => 1,
        2 => 4,
        3 => 5,
        4 => 6,
        _ => {
            return Err(period_timestamp_error(
                "business-day remainder out of range",
            ));
        }
    };
    let calendar_days = week
        .checked_mul(7)
        .and_then(|days| days.checked_add(day_offset))
        .ok_or_else(|| period_timestamp_error("business-day ordinal overflow"))?;
    period_add_days(period_epoch_date(1970, 1, 1)?, calendar_days)
}

fn period_start_nanos(period: Period) -> Result<i64, IndexError> {
    match period.freq {
        PeriodFreq::Annual => {
            let month_ordinal = period
                .ordinal
                .checked_mul(12)
                .ok_or_else(|| period_timestamp_error("annual ordinal overflow"))?;
            period_date_to_nanos(period_month_start(month_ordinal)?)
        }
        PeriodFreq::Quarterly => {
            let month_ordinal = period
                .ordinal
                .checked_mul(3)
                .ok_or_else(|| period_timestamp_error("quarterly ordinal overflow"))?;
            period_date_to_nanos(period_month_start(month_ordinal)?)
        }
        PeriodFreq::Monthly => period_date_to_nanos(period_month_start(period.ordinal)?),
        PeriodFreq::Weekly => {
            let base = period_epoch_date(1969, 12, 22)?;
            let days = period
                .ordinal
                .checked_mul(7)
                .ok_or_else(|| period_timestamp_error("weekly ordinal overflow"))?;
            period_date_to_nanos(period_add_days(base, days)?)
        }
        PeriodFreq::Daily => {
            let base = period_epoch_date(1970, 1, 1)?;
            period_date_to_nanos(period_add_days(base, period.ordinal)?)
        }
        PeriodFreq::Business => period_date_to_nanos(period_business_date(period.ordinal)?),
        PeriodFreq::Hourly => period
            .ordinal
            .checked_mul(Timedelta::NANOS_PER_HOUR)
            .ok_or_else(|| period_timestamp_error("hourly ordinal overflow")),
        PeriodFreq::Minutely => period
            .ordinal
            .checked_mul(Timedelta::NANOS_PER_MIN)
            .ok_or_else(|| period_timestamp_error("minutely ordinal overflow")),
        PeriodFreq::Secondly => period
            .ordinal
            .checked_mul(Timedelta::NANOS_PER_SEC)
            .ok_or_else(|| period_timestamp_error("secondly ordinal overflow")),
        _ => Err(period_timestamp_error("unsupported period frequency")),
    }
}

fn period_next_start_nanos(period: Period) -> Result<i64, IndexError> {
    let next = Period {
        ordinal: period
            .ordinal
            .checked_add(1)
            .ok_or_else(|| period_timestamp_error("period ordinal overflow"))?,
        freq: period.freq,
    };
    period_start_nanos(next)
}

fn period_end_nanos(period: Period) -> Result<i64, IndexError> {
    period_checked_add_nanos(period_next_start_nanos(period)?, -1)
}

fn period_boundary_nanos(period: Period, boundary: PeriodBoundary) -> Result<i64, IndexError> {
    match boundary {
        PeriodBoundary::Start => period_start_nanos(period),
        PeriodBoundary::End => period_end_nanos(period),
    }
}

fn parse_period_boundary_how(how: &str, context: &str) -> Result<PeriodBoundary, IndexError> {
    match how.trim().to_ascii_lowercase().as_str() {
        "" | "e" | "end" | "finish" => Ok(PeriodBoundary::End),
        "s" | "start" | "begin" | "b" => Ok(PeriodBoundary::Start),
        other => Err(IndexError::InvalidArgument(format!(
            "{context} how must be 'start' or 'end', got {other:?}"
        ))),
    }
}

fn period_qyear(period: Period) -> Result<i32, IndexError> {
    let end_nanos = period_end_nanos(period)?;
    datetime_nanos_to_date(end_nanos)
        .map(|date| date.year())
        .map_err(period_date_error)
}

#[derive(Debug, Clone, Copy)]
pub struct PeriodFields<'a> {
    pub year: &'a [i32],
    pub quarter: Option<&'a [u32]>,
    pub month: Option<&'a [u32]>,
    pub day: Option<&'a [u32]>,
    pub hour: Option<&'a [u32]>,
    pub minute: Option<&'a [u32]>,
    pub second: Option<&'a [u32]>,
    pub freq: Option<PeriodFreq>,
}

impl<'a> PeriodFields<'a> {
    #[must_use]
    pub const fn new(year: &'a [i32]) -> Self {
        Self {
            year,
            quarter: None,
            month: None,
            day: None,
            hour: None,
            minute: None,
            second: None,
            freq: None,
        }
    }
}

fn period_fields_error(message: impl Into<String>) -> IndexError {
    IndexError::InvalidArgument(format!(
        "PeriodIndex.from_fields failed: {}",
        message.into()
    ))
}

fn period_fields_freq(fields: &PeriodFields<'_>) -> Result<PeriodFreq, IndexError> {
    let freq = fields
        .freq
        .or_else(|| fields.quarter.map(|_| PeriodFreq::Quarterly))
        .ok_or_else(|| {
            period_fields_error("freq is required unless quarter fields imply quarterly periods")
        })?;
    if fields.quarter.is_some() && freq != PeriodFreq::Quarterly {
        return Err(period_fields_error(
            "quarter fields require quarterly frequency",
        ));
    }
    Ok(freq)
}

fn validate_period_field_len(
    name: &str,
    values: Option<&[u32]>,
    expected: usize,
) -> Result<(), IndexError> {
    if values.is_some_and(|items| items.len() != expected) {
        return Err(period_fields_error(format!(
            "Mismatched Period array lengths for {name}"
        )));
    }
    Ok(())
}

fn validate_period_fields(fields: &PeriodFields<'_>) -> Result<(), IndexError> {
    let expected = fields.year.len();
    validate_period_field_len("quarter", fields.quarter, expected)?;
    validate_period_field_len("month", fields.month, expected)?;
    validate_period_field_len("day", fields.day, expected)?;
    validate_period_field_len("hour", fields.hour, expected)?;
    validate_period_field_len("minute", fields.minute, expected)?;
    validate_period_field_len("second", fields.second, expected)
}

fn period_field_value(values: Option<&[u32]>, position: usize, default: u32) -> u32 {
    values
        .and_then(|items| items.get(position).copied())
        .unwrap_or(default)
}

fn required_period_field(
    values: Option<&[u32]>,
    name: &str,
    position: usize,
) -> Result<u32, IndexError> {
    values
        .and_then(|items| items.get(position).copied())
        .ok_or_else(|| period_fields_error(format!("{name} fields are required")))
}

fn quarter_start_month(quarter: u32) -> Result<u32, IndexError> {
    if (1..=4).contains(&quarter) {
        Ok((quarter - 1) * 3 + 1)
    } else {
        Err(period_fields_error(format!(
            "quarter must be in 1..=4, got {quarter}"
        )))
    }
}

fn period_from_fields_at(
    fields: &PeriodFields<'_>,
    freq: PeriodFreq,
    position: usize,
) -> Result<Period, IndexError> {
    let year = fields
        .year
        .get(position)
        .copied()
        .ok_or_else(|| period_fields_error("year fields are required"))?;
    let month = if freq == PeriodFreq::Quarterly {
        if let Some(quarters) = fields.quarter {
            let quarter = quarters
                .get(position)
                .copied()
                .ok_or_else(|| period_fields_error("quarter fields are required"))?;
            quarter_start_month(quarter)?
        } else {
            required_period_field(fields.month, "month", position)?
        }
    } else {
        if fields.quarter.is_some() && fields.month.is_none() {
            return Err(period_fields_error(
                "quarter fields require quarterly frequency unless month is also supplied",
            ));
        }
        required_period_field(fields.month, "month", position)?
    };
    let day = if matches!(
        freq,
        PeriodFreq::Annual | PeriodFreq::Quarterly | PeriodFreq::Monthly
    ) {
        1
    } else {
        period_field_value(fields.day, position, 1)
    };
    let hour = if matches!(
        freq,
        PeriodFreq::Hourly | PeriodFreq::Minutely | PeriodFreq::Secondly
    ) {
        period_field_value(fields.hour, position, 0)
    } else {
        0
    };
    let minute = if matches!(freq, PeriodFreq::Minutely | PeriodFreq::Secondly) {
        period_field_value(fields.minute, position, 0)
    } else {
        0
    };
    let second = if freq == PeriodFreq::Secondly {
        period_field_value(fields.second, position, 0)
    } else {
        0
    };
    let date = chrono::NaiveDate::from_ymd_opt(year, month, day)
        .ok_or_else(|| period_fields_error("invalid year/month/day combination"))?;
    let time = chrono::NaiveTime::from_hms_opt(hour, minute, second)
        .ok_or_else(|| period_fields_error("invalid hour/minute/second combination"))?;
    let nanos = date_and_time_to_nanos(date, time_to_nanos(time)).map_err(period_date_error)?;
    datetime_period_ordinal(nanos, freq).map(|ordinal| Period::new(ordinal, freq))
}

fn ensure_index_kind(
    index: &Index,
    predicate: impl Fn(&IndexLabel) -> bool,
    kind: &str,
) -> Result<(), IndexError> {
    if index.labels().iter().all(predicate) {
        Ok(())
    } else {
        Err(IndexError::InvalidArgument(format!(
            "{kind} requires homogeneous {kind} labels"
        )))
    }
}

/// Public pandas-style datetime index wrapper.
///
/// The canonical storage remains [`Index`] with `Datetime64` labels so existing
/// DataFrame/Series alignment code keeps one representation. This wrapper adds
/// the type-level public surface pandas users expect (`DatetimeIndex`) and a
/// small first slice of datetime accessors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatetimeIndex {
    index: Index,
}

impl DatetimeIndex {
    #[must_use]
    pub fn new(nanos: Vec<i64>) -> Self {
        Self {
            index: Index::from_datetime64(nanos),
        }
    }

    pub fn from_index(index: Index) -> Result<Self, IndexError> {
        ensure_index_kind(
            &index,
            |label| matches!(label, IndexLabel::Datetime64(_)),
            "DatetimeIndex",
        )?;
        Ok(Self { index })
    }

    #[must_use]
    pub fn as_index(&self) -> &Index {
        &self.index
    }

    #[must_use]
    pub fn into_index(self) -> Index {
        self.index
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.index.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.index.name()
    }

    #[must_use]
    pub fn set_name(&self, name: &str) -> Self {
        Self {
            index: self.index.set_name(name),
        }
    }

    #[must_use]
    pub fn set_names(&self, name: Option<&str>) -> Self {
        Self {
            index: self.index.set_names(name),
        }
    }

    #[must_use]
    pub fn rename_index(&self, name: Option<&str>) -> Self {
        self.set_names(name)
    }

    #[must_use]
    pub fn names(&self) -> Vec<Option<String>> {
        self.index.names()
    }

    #[must_use]
    pub fn copy(&self) -> Self {
        self.clone()
    }

    #[must_use]
    pub fn shape(&self) -> (usize,) {
        self.index.shape()
    }

    #[must_use]
    pub fn size(&self) -> usize {
        self.index.size()
    }

    #[must_use]
    pub fn empty(&self) -> bool {
        self.index.empty()
    }

    #[must_use]
    pub fn dtype(&self) -> &'static str {
        "datetime64[ns]"
    }

    #[must_use]
    pub fn dtypes(&self) -> Vec<&'static str> {
        vec![self.dtype()]
    }

    #[must_use]
    pub fn memory_usage(&self, deep: bool) -> usize {
        self.index.memory_usage(deep)
    }

    #[must_use]
    pub fn nbytes(&self) -> usize {
        self.index.nbytes()
    }

    #[must_use]
    pub fn hasnans(&self) -> bool {
        self.index.hasnans()
    }

    #[must_use]
    pub fn isna(&self) -> Vec<bool> {
        self.index.isna()
    }

    #[must_use]
    pub fn notna(&self) -> Vec<bool> {
        self.index.notna()
    }

    #[must_use]
    pub fn is_unique(&self) -> bool {
        self.index.is_unique()
    }

    #[must_use]
    pub fn has_duplicates(&self) -> bool {
        self.index.has_duplicates()
    }

    #[must_use]
    pub fn is_monotonic_increasing(&self) -> bool {
        self.index.is_monotonic_increasing()
    }

    #[must_use]
    pub fn is_monotonic(&self) -> bool {
        self.index.is_monotonic()
    }

    #[must_use]
    pub fn is_monotonic_decreasing(&self) -> bool {
        self.index.is_monotonic_decreasing()
    }

    #[must_use]
    pub fn nunique(&self) -> usize {
        self.index.nunique()
    }

    #[must_use]
    pub fn nunique_with_dropna(&self, dropna: bool) -> usize {
        self.index.nunique_with_dropna(dropna)
    }

    #[must_use]
    pub fn ndim(&self) -> usize {
        self.index.ndim()
    }

    pub fn item(&self) -> Result<Option<i64>, IndexError> {
        match self.index.item()? {
            IndexLabel::Datetime64(nanos) if nanos != i64::MIN => Ok(Some(nanos)),
            IndexLabel::Datetime64(_) => Ok(None),
            label => Err(IndexError::InvalidArgument(format!(
                "DatetimeIndex item must be datetime64, got {label}"
            ))),
        }
    }

    #[must_use]
    pub fn is_(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }

    #[must_use]
    pub fn equals(&self, other: &Self) -> bool {
        self.index.equals(&other.index)
    }

    #[must_use]
    pub fn identical(&self, other: &Self) -> bool {
        self.index.identical(&other.index)
    }

    #[must_use]
    pub fn holds_integer(&self) -> bool {
        false
    }

    #[must_use]
    pub fn inferred_type(&self) -> &'static str {
        "datetime64"
    }

    #[must_use]
    pub fn is_boolean(&self) -> bool {
        false
    }

    #[must_use]
    pub fn is_categorical(&self) -> bool {
        false
    }

    #[must_use]
    pub fn is_floating(&self) -> bool {
        false
    }

    #[must_use]
    pub fn is_integer(&self) -> bool {
        false
    }

    #[must_use]
    pub fn is_interval(&self) -> bool {
        false
    }

    #[must_use]
    pub fn is_numeric(&self) -> bool {
        false
    }

    #[must_use]
    pub fn is_object(&self) -> bool {
        false
    }

    #[must_use]
    pub fn nanos(&self) -> Vec<Option<i64>> {
        self.index
            .labels()
            .iter()
            .map(|label| match label {
                IndexLabel::Datetime64(nanos) if *nanos != i64::MIN => Some(*nanos),
                IndexLabel::Int64(_)
                | IndexLabel::Float64(_)
                | IndexLabel::Bool(_)
                | IndexLabel::Utf8(_)
                | IndexLabel::Timedelta64(_)
                | IndexLabel::Datetime64(_)
                | IndexLabel::Null(_) => None,
            })
            .collect()
    }

    #[must_use]
    pub fn values(&self) -> Vec<Option<i64>> {
        self.nanos()
    }

    #[must_use]
    pub fn to_list(&self) -> Vec<Option<i64>> {
        self.nanos()
    }

    #[must_use]
    pub fn tolist(&self) -> Vec<Option<i64>> {
        self.to_list()
    }

    #[must_use]
    pub fn to_numpy(&self) -> Vec<Option<i64>> {
        self.nanos()
    }

    #[must_use]
    pub fn array(&self) -> Vec<Option<i64>> {
        self.nanos()
    }

    /// Underlying nanoseconds-since-epoch, matching `pd.DatetimeIndex.asi8`.
    /// NAT is preserved as `i64::MIN` to match the on-disk sentinel.
    #[must_use]
    pub fn asi8(&self) -> Vec<i64> {
        self.index
            .labels()
            .iter()
            .map(|label| match label {
                IndexLabel::Datetime64(nanos) => *nanos,
                IndexLabel::Int64(_)
                | IndexLabel::Float64(_)
                | IndexLabel::Bool(_)
                | IndexLabel::Utf8(_)
                | IndexLabel::Timedelta64(_)
                | IndexLabel::Null(_) => i64::MIN,
            })
            .collect()
    }

    /// Convert datetime labels to period ordinals at the requested frequency,
    /// matching `pd.DatetimeIndex.to_period(freq)` for supported fixed
    /// calendar frequencies.
    pub fn to_period(&self, freq: &str) -> Result<PeriodIndex, IndexError> {
        let period_freq = PeriodFreq::parse(freq).ok_or_else(|| {
            IndexError::InvalidArgument(format!("to_period: unsupported frequency '{freq}'"))
        })?;
        let periods = self
            .index
            .labels()
            .iter()
            .map(|label| match label {
                IndexLabel::Datetime64(nanos) => datetime_nanos_to_period(*nanos, period_freq),
                other => Err(IndexError::InvalidArgument(format!(
                    "to_period requires DatetimeIndex labels, got {other:?}"
                ))),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut out = PeriodIndex::new(periods);
        if let Some(name) = self.name() {
            out = out.set_name(name);
        }
        Ok(out)
    }

    /// Format each timestamp using a chrono format string, matching
    /// `pd.DatetimeIndex.strftime(format)`. NAT propagates as `None`.
    #[must_use]
    pub fn strftime(&self, format: &str) -> Vec<Option<String>> {
        map_datetime_labels(self.index.labels(), |dt| dt.format(format).to_string())
    }

    /// Position of the maximum label, matching `pd.DatetimeIndex.argmax()`.
    /// Pandas returns the *first* tied position and skips NAT entries; this
    /// method walks the labels itself to match that ordering exactly. Empty
    /// indexes (or all-NAT indexes) raise pandas-style `ValueError` mirrored
    /// as [`IndexError::InvalidArgument`].
    pub fn argmax(&self) -> Result<usize, IndexError> {
        let labels = self.index.labels();
        let mut best: Option<usize> = None;
        for (i, label) in labels.iter().enumerate() {
            let nanos = match label {
                IndexLabel::Datetime64(n) if *n != i64::MIN => *n,
                _ => continue,
            };
            best = Some(match best {
                Some(b) => match labels[b] {
                    IndexLabel::Datetime64(prev) if nanos > prev => i,
                    _ => b,
                },
                None => i,
            });
        }
        best.ok_or_else(|| {
            IndexError::InvalidArgument("attempt to get argmax of an empty sequence".to_owned())
        })
    }

    /// Position of the minimum label, matching `pd.DatetimeIndex.argmin()`.
    /// Returns the first-tied position and skips NAT to match pandas semantics.
    pub fn argmin(&self) -> Result<usize, IndexError> {
        let labels = self.index.labels();
        let mut best: Option<usize> = None;
        for (i, label) in labels.iter().enumerate() {
            let nanos = match label {
                IndexLabel::Datetime64(n) if *n != i64::MIN => *n,
                _ => continue,
            };
            best = Some(match best {
                Some(b) => match labels[b] {
                    IndexLabel::Datetime64(prev) if nanos < prev => i,
                    _ => b,
                },
                None => i,
            });
        }
        best.ok_or_else(|| {
            IndexError::InvalidArgument("attempt to get argmin of an empty sequence".to_owned())
        })
    }

    /// Positions that would sort the labels ascending, matching
    /// `pd.DatetimeIndex.argsort()`.
    #[must_use]
    pub fn argsort(&self) -> Vec<usize> {
        self.index.argsort()
    }

    /// First-seen unique labels, matching `pd.DatetimeIndex.unique()`.
    /// Returns a new DatetimeIndex.
    pub fn unique(&self) -> Result<Self, IndexError> {
        Self::from_index(self.index.unique())
    }

    /// Identity-stable factorization, matching `pd.DatetimeIndex.factorize()`.
    /// Returns `(codes, uniques)` where `uniques` is rebuilt as DatetimeIndex.
    pub fn factorize(&self) -> Result<(Vec<isize>, Self), IndexError> {
        let (codes, uniques) = self.index.factorize();
        Ok((codes, Self::from_index(uniques)?))
    }

    /// Value counts, matching `pd.DatetimeIndex.value_counts()`.
    #[must_use]
    pub fn value_counts(&self) -> Vec<(IndexLabel, usize)> {
        self.index.value_counts()
    }

    /// Duplicate mask per position, matching `pd.DatetimeIndex.duplicated(keep)`.
    #[must_use]
    pub fn duplicated(&self, keep: DuplicateKeep) -> Vec<bool> {
        self.index.duplicated(keep)
    }

    /// Drop duplicate labels, matching `pd.DatetimeIndex.drop_duplicates()`.
    pub fn drop_duplicates(&self) -> Result<Self, IndexError> {
        Self::from_index(self.index.drop_duplicates())
    }

    /// Pick labels at the given positions, matching `pd.DatetimeIndex.take()`.
    /// Out-of-bounds positions raise [`IndexError::OutOfBounds`].
    pub fn take(&self, positions: &[usize]) -> Result<Self, IndexError> {
        let labels = self.index.labels();
        for &p in positions {
            if p >= labels.len() {
                return Err(IndexError::OutOfBounds {
                    position: p,
                    length: labels.len(),
                });
            }
        }
        let nanos: Vec<i64> = positions
            .iter()
            .map(|&p| match labels[p] {
                IndexLabel::Datetime64(n) => n,
                _ => i64::MIN,
            })
            .collect();
        let mut out = Self::new(nanos);
        if let Some(name) = self.name() {
            out = out.set_name(name);
        }
        Ok(out)
    }

    /// Repeat each label `repeats` times, matching `pd.DatetimeIndex.repeat()`.
    #[must_use]
    pub fn repeat(&self, repeats: usize) -> Self {
        let mut out = Vec::with_capacity(self.len() * repeats);
        for label in self.index.labels() {
            if let IndexLabel::Datetime64(n) = label {
                for _ in 0..repeats {
                    out.push(*n);
                }
            }
        }
        let mut result = Self::new(out);
        if let Some(name) = self.name() {
            result = result.set_name(name);
        }
        result
    }

    /// Per-position membership mask, matching `pd.DatetimeIndex.isin(values)`.
    /// `values` is interpreted as a slice of nanoseconds-since-epoch; pass
    /// `i64::MIN` to test for NAT.
    #[must_use]
    pub fn isin(&self, values: &[i64]) -> Vec<bool> {
        let needle: FxHashSet<i64> = values.iter().copied().collect();
        self.index
            .labels()
            .iter()
            .map(|label| match label {
                IndexLabel::Datetime64(n) => needle.contains(n),
                _ => false,
            })
            .collect()
    }

    /// Concatenate with another DatetimeIndex, matching
    /// `pd.DatetimeIndex.append(other)`. The index name is preserved when
    /// both operands share it; otherwise pandas drops the name.
    #[must_use]
    pub fn append(&self, other: &Self) -> Self {
        let mut nanos: Vec<i64> = self
            .index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Datetime64(n) => Some(*n),
                _ => None,
            })
            .collect();
        nanos.extend(other.index.labels().iter().filter_map(|label| match label {
            IndexLabel::Datetime64(n) => Some(*n),
            _ => None,
        }));
        let mut out = Self::new(nanos);
        if let Some(name) = self.name().filter(|_| self.name() == other.name()) {
            out = out.set_name(name);
        }
        out
    }

    /// Minimum non-NAT label, matching `pd.DatetimeIndex.min()`.
    /// Returns `None` for empty or all-NAT inputs to mirror pandas' NaT.
    #[must_use]
    pub fn min(&self) -> Option<i64> {
        self.index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Datetime64(n) if *n != i64::MIN => Some(*n),
                _ => None,
            })
            .min()
    }

    /// Shift each label by `periods` units of `freq_nanos`, matching
    /// `pd.DatetimeIndex.shift(periods, freq)` once `freq` has been
    /// resolved to a nanosecond duration. NAT propagates as NAT;
    /// arithmetic overflow saturates.
    #[must_use]
    pub fn shift(&self, periods: i64, freq_nanos: i64) -> Self {
        let delta = periods.saturating_mul(freq_nanos);
        let nanos: Vec<i64> = self
            .index
            .labels()
            .iter()
            .map(|label| match label {
                IndexLabel::Datetime64(n) if *n != i64::MIN => n.saturating_add(delta),
                _ => i64::MIN,
            })
            .collect();
        let mut out = Self::new(nanos);
        if let Some(name) = self.name() {
            out = out.set_name(name);
        }
        out
    }

    /// Positional first differences, matching `pd.DatetimeIndex.diff()`.
    /// Datetime deltas materialize as a TimedeltaIndex; NAT inputs propagate
    /// to NAT outputs, and signed `periods` follows pandas' forward/backward
    /// lookup direction.
    #[must_use]
    pub fn diff(&self, periods: i64) -> TimedeltaIndex {
        let labels = self.index.labels();
        optional_diffs_to_timedelta_index(
            positional_diff(labels.len(), periods, |current, previous| {
                match (&labels[current], &labels[previous]) {
                    (
                        IndexLabel::Datetime64(current_nanos),
                        IndexLabel::Datetime64(previous_nanos),
                    ) if *current_nanos != i64::MIN && *previous_nanos != i64::MIN => {
                        current_nanos.checked_sub(*previous_nanos)
                    }
                    _ => None,
                }
            }),
            self.name(),
        )
    }

    fn round_fixed_freq(&self, freq: &str, mode: TemporalRoundMode) -> Result<Self, IndexError> {
        let unit_nanos = parse_fixed_temporal_freq(freq, "DatetimeIndex rounding")?;
        let nanos: Vec<i64> = self
            .index
            .labels()
            .iter()
            .map(|label| match label {
                IndexLabel::Datetime64(n) if *n != i64::MIN => {
                    round_nanos_to_unit(*n, unit_nanos, mode)
                }
                _ => i64::MIN,
            })
            .collect();
        let mut out = Self::new(nanos);
        if let Some(name) = self.name() {
            out = out.set_name(name);
        }
        Ok(out)
    }

    /// Round timestamps down to a fixed pandas frequency.
    pub fn floor(&self, freq: &str) -> Result<Self, IndexError> {
        self.round_fixed_freq(freq, TemporalRoundMode::Floor)
    }

    /// Round timestamps up to a fixed pandas frequency.
    pub fn ceil(&self, freq: &str) -> Result<Self, IndexError> {
        self.round_fixed_freq(freq, TemporalRoundMode::Ceil)
    }

    /// Round timestamps to the nearest fixed pandas frequency, using half-even ties.
    pub fn round(&self, freq: &str) -> Result<Self, IndexError> {
        self.round_fixed_freq(freq, TemporalRoundMode::Round)
    }

    /// Validate the frequency and return a clone, matching pandas DatetimeIndex.snap.
    pub fn snap(&self, freq: &str) -> Result<Self, IndexError> {
        parse_fixed_temporal_freq(freq, "DatetimeIndex.snap")?;
        Ok(self.clone())
    }

    /// Average non-NAT label as nanoseconds-since-epoch, matching
    /// `pd.DatetimeIndex.mean()`. Empty / all-NAT returns `None`.
    /// Sum is computed in `i128` to avoid overflow.
    #[must_use]
    pub fn mean(&self) -> Option<i64> {
        let mut total: i128 = 0;
        let mut count: i128 = 0;
        for label in self.index.labels() {
            if let IndexLabel::Datetime64(n) = label
                && *n != i64::MIN
            {
                total += i128::from(*n);
                count += 1;
            }
        }
        if count == 0 {
            return None;
        }
        i64::try_from(total / count).ok()
    }

    /// Sample variance over non-NAT labels in nanoseconds-squared,
    /// matching `pd.DatetimeIndex.var(ddof=1)`. Returns `None` for
    /// fewer than two non-NAT entries.
    #[must_use]
    pub fn var(&self) -> Option<f64> {
        let nanos: Vec<f64> = self
            .index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Datetime64(n) if *n != i64::MIN => Some(*n as f64),
                _ => None,
            })
            .collect();
        if nanos.len() < 2 {
            return None;
        }
        let mean = nanos.iter().sum::<f64>() / nanos.len() as f64;
        Some(nanos.iter().map(|n| (n - mean).powi(2)).sum::<f64>() / (nanos.len() as f64 - 1.0))
    }

    /// Sample standard deviation of non-NAT labels in nanoseconds,
    /// matching `pd.DatetimeIndex.std(ddof=1)`. Returns `None` for
    /// fewer than two non-NAT entries.
    #[must_use]
    pub fn std(&self) -> Option<i64> {
        let nanos: Vec<f64> = self
            .index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Datetime64(n) if *n != i64::MIN => Some(*n as f64),
                _ => None,
            })
            .collect();
        if nanos.len() < 2 {
            return None;
        }
        let mean = nanos.iter().sum::<f64>() / nanos.len() as f64;
        let var =
            nanos.iter().map(|n| (n - mean).powi(2)).sum::<f64>() / (nanos.len() as f64 - 1.0);
        Some(var.sqrt() as i64)
    }

    /// Median non-NAT label, matching `pd.DatetimeIndex.median()`. Empty
    /// returns None. For an even-length non-NAT subset, returns the
    /// average of the two middle values.
    #[must_use]
    pub fn median(&self) -> Option<i64> {
        let mut nanos: Vec<i64> = self
            .index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Datetime64(n) if *n != i64::MIN => Some(*n),
                _ => None,
            })
            .collect();
        if nanos.is_empty() {
            return None;
        }
        nanos.sort_unstable();
        let mid = nanos.len() / 2;
        if nanos.len() % 2 == 1 {
            Some(nanos[mid])
        } else {
            let total = i128::from(nanos[mid - 1]) + i128::from(nanos[mid]);
            i64::try_from(total / 2).ok()
        }
    }

    /// Maximum non-NAT label, matching `pd.DatetimeIndex.max()`.
    #[must_use]
    pub fn max(&self) -> Option<i64> {
        self.index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Datetime64(n) if *n != i64::MIN => Some(*n),
                _ => None,
            })
            .max()
    }

    /// Labels present in both indexes, matching
    /// `pd.DatetimeIndex.intersection(other)`. Preserves first-seen order
    /// from `self`.
    #[must_use]
    pub fn intersection(&self, other: &Self) -> Self {
        let other_set: FxHashSet<i64> = other
            .index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Datetime64(n) => Some(*n),
                _ => None,
            })
            .collect();
        let mut seen = FxHashSet::<i64>::default();
        let nanos: Vec<i64> = self
            .index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Datetime64(n) if other_set.contains(n) && seen.insert(*n) => Some(*n),
                _ => None,
            })
            .collect();
        let mut out = Self::new(nanos);
        if let Some(name) = self.name().filter(|_| self.name() == other.name()) {
            out = out.set_name(name);
        }
        out
    }

    /// Labels from self followed by labels from other not already present,
    /// matching `pd.DatetimeIndex.union(other)`.
    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        let mut seen = FxHashSet::<i64>::default();
        let mut nanos: Vec<i64> = Vec::new();
        for label in self
            .index
            .labels()
            .iter()
            .chain(other.index.labels().iter())
        {
            if let IndexLabel::Datetime64(n) = label
                && seen.insert(*n)
            {
                nanos.push(*n);
            }
        }
        let mut out = Self::new(nanos);
        if let Some(name) = self.name().filter(|_| self.name() == other.name()) {
            out = out.set_name(name);
        }
        out
    }

    /// Labels in self not in other, matching
    /// `pd.DatetimeIndex.difference(other)`.
    #[must_use]
    pub fn difference(&self, other: &Self) -> Self {
        let other_set: FxHashSet<i64> = other
            .index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Datetime64(n) => Some(*n),
                _ => None,
            })
            .collect();
        let mut seen = FxHashSet::<i64>::default();
        let nanos: Vec<i64> = self
            .index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Datetime64(n) if !other_set.contains(n) && seen.insert(*n) => Some(*n),
                _ => None,
            })
            .collect();
        let mut out = Self::new(nanos);
        // Per br-frankenpandas-6r1lq: difference is asymmetric — pandas
        // always preserves self.name (unlike union/intersection which use
        // shared_name).
        if let Some(name) = self.name() {
            out = out.set_name(name);
        }
        out
    }

    /// Labels in either but not both, matching
    /// `pd.DatetimeIndex.symmetric_difference(other)`.
    #[must_use]
    pub fn symmetric_difference(&self, other: &Self) -> Self {
        let self_set: FxHashSet<i64> = self
            .index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Datetime64(n) => Some(*n),
                _ => None,
            })
            .collect();
        let other_set: FxHashSet<i64> = other
            .index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Datetime64(n) => Some(*n),
                _ => None,
            })
            .collect();
        let mut seen = FxHashSet::<i64>::default();
        let mut nanos: Vec<i64> = Vec::new();
        for label in self.index.labels() {
            if let IndexLabel::Datetime64(n) = label
                && !other_set.contains(n)
                && seen.insert(*n)
            {
                nanos.push(*n);
            }
        }
        for label in other.index.labels() {
            if let IndexLabel::Datetime64(n) = label
                && !self_set.contains(n)
                && seen.insert(*n)
            {
                nanos.push(*n);
            }
        }
        let mut out = Self::new(nanos);
        if let Some(name) = self.name().filter(|_| self.name() == other.name()) {
            out = out.set_name(name);
        }
        out
    }

    /// Sort labels ascending, matching `pd.DatetimeIndex.sort_values()`.
    /// NAT sorts first because the underlying sentinel is `i64::MIN`,
    /// matching pandas' `na_position='first'` default for datetime indexes.
    #[must_use]
    pub fn sort_values(&self) -> Self {
        let mut nanos: Vec<i64> = self
            .index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Datetime64(n) => Some(*n),
                _ => None,
            })
            .collect();
        nanos.sort_unstable();
        let mut out = Self::new(nanos);
        if let Some(name) = self.name() {
            out = out.set_name(name);
        }
        out
    }

    /// Alias for `sort_values`, matching `pd.DatetimeIndex.sort()`.
    #[must_use]
    pub fn sort(&self) -> Self {
        self.sort_values()
    }

    /// Remove the label at the given position, matching
    /// `pd.DatetimeIndex.delete(loc)`.
    pub fn delete(&self, loc: usize) -> Result<Self, IndexError> {
        let labels = self.index.labels();
        if loc >= labels.len() {
            return Err(IndexError::OutOfBounds {
                position: loc,
                length: labels.len(),
            });
        }
        let nanos: Vec<i64> = labels
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != loc)
            .filter_map(|(_, label)| match label {
                IndexLabel::Datetime64(n) => Some(*n),
                _ => None,
            })
            .collect();
        let mut out = Self::new(nanos);
        if let Some(name) = self.name() {
            out = out.set_name(name);
        }
        Ok(out)
    }

    /// Replace positions where `cond` is `false` with `other`, matching
    /// `pd.DatetimeIndex.where(cond, other)`. Pass `i64::MIN` to insert
    /// NAT.
    pub fn r#where(&self, cond: &[bool], other: i64) -> Result<Self, IndexError> {
        let labels = self.index.labels();
        if cond.len() != labels.len() {
            return Err(IndexError::LengthMismatch {
                expected: labels.len(),
                actual: cond.len(),
                context: "where: cond length must match index length".to_owned(),
            });
        }
        let nanos: Vec<i64> = labels
            .iter()
            .zip(cond.iter())
            .map(|(label, &keep)| {
                if keep {
                    match label {
                        IndexLabel::Datetime64(n) => *n,
                        _ => i64::MIN,
                    }
                } else {
                    other
                }
            })
            .collect();
        let mut out = Self::new(nanos);
        if let Some(name) = self.name() {
            out = out.set_name(name);
        }
        Ok(out)
    }

    /// Replace positions where `mask` is `true` with `value`, matching
    /// `pd.DatetimeIndex.putmask(mask, value)`. The complement of `where`.
    pub fn putmask(&self, mask: &[bool], value: i64) -> Result<Self, IndexError> {
        let labels = self.index.labels();
        if mask.len() != labels.len() {
            return Err(IndexError::LengthMismatch {
                expected: labels.len(),
                actual: mask.len(),
                context: "putmask: mask length must match index length".to_owned(),
            });
        }
        let nanos: Vec<i64> = labels
            .iter()
            .zip(mask.iter())
            .map(|(label, &replace)| {
                if replace {
                    value
                } else {
                    match label {
                        IndexLabel::Datetime64(n) => *n,
                        _ => i64::MIN,
                    }
                }
            })
            .collect();
        let mut out = Self::new(nanos);
        if let Some(name) = self.name() {
            out = out.set_name(name);
        }
        Ok(out)
    }

    /// Binary-search insertion position, matching
    /// `pd.DatetimeIndex.searchsorted(value, side)`. The needle is the
    /// nanoseconds-since-epoch value to locate; pandas behavior on NAT
    /// needles is to raise, mirrored as
    /// [`IndexError::InvalidArgument("searchsorted: needle cannot be missing")`].
    pub fn searchsorted(&self, value: i64, side: &str) -> Result<usize, IndexError> {
        self.index
            .searchsorted(&IndexLabel::Datetime64(value), side)
    }

    /// Convert each label to a `chrono::DateTime<Utc>`, matching
    /// `pd.DatetimeIndex.to_pydatetime()`. NAT propagates as `None`.
    #[must_use]
    pub fn to_pydatetime(&self) -> Vec<Option<chrono::DateTime<chrono::Utc>>> {
        self.index
            .labels()
            .iter()
            .map(|label| match label {
                IndexLabel::Datetime64(nanos) => datetime_from_nanos(*nanos),
                _ => None,
            })
            .collect()
    }

    /// Insert `value` at position `loc`, matching
    /// `pd.DatetimeIndex.insert(loc, value)`. `loc == len()` appends;
    /// `loc > len()` raises [`IndexError::OutOfBounds`].
    pub fn insert(&self, loc: usize, value: i64) -> Result<Self, IndexError> {
        let labels = self.index.labels();
        if loc > labels.len() {
            return Err(IndexError::OutOfBounds {
                position: loc,
                length: labels.len(),
            });
        }
        let mut nanos: Vec<i64> = labels
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Datetime64(n) => Some(*n),
                _ => None,
            })
            .collect();
        nanos.insert(loc, value);
        let mut out = Self::new(nanos);
        if let Some(name) = self.name() {
            out = out.set_name(name);
        }
        Ok(out)
    }

    /// Stringify each label, matching `pd.DatetimeIndex.format()`.
    /// Non-NAT labels render as the chrono RFC3339 timestamp; NAT
    /// renders as the `NaT` literal pandas uses.
    #[must_use]
    pub fn format(&self) -> Vec<String> {
        self.index
            .labels()
            .iter()
            .map(|label| match label {
                IndexLabel::Datetime64(nanos) => match datetime_from_nanos(*nanos) {
                    Some(dt) => dt.to_rfc3339(),
                    None => "NaT".to_owned(),
                },
                _ => "NaT".to_owned(),
            })
            .collect()
    }

    /// Replace NAT positions with `value`, matching
    /// `pd.DatetimeIndex.fillna(value)`. Preserves the index name.
    #[must_use]
    pub fn fillna(&self, value: i64) -> Self {
        let nanos: Vec<i64> = self
            .index
            .labels()
            .iter()
            .map(|label| match label {
                IndexLabel::Datetime64(n) if *n != i64::MIN => *n,
                _ => value,
            })
            .collect();
        let mut out = Self::new(nanos);
        if let Some(name) = self.name() {
            out = out.set_name(name);
        }
        out
    }

    /// Alias for [`isna`], matching `pd.DatetimeIndex.isnull()`.
    #[must_use]
    pub fn isnull(&self) -> Vec<bool> {
        self.isna()
    }

    /// Alias for [`notna`], matching `pd.DatetimeIndex.notnull()`.
    #[must_use]
    pub fn notnull(&self) -> Vec<bool> {
        self.notna()
    }

    /// Calendar date part of each label, matching `pd.DatetimeIndex.date`.
    #[must_use]
    pub fn date(&self) -> Vec<Option<chrono::NaiveDate>> {
        map_datetime_labels(self.index.labels(), |dt| dt.date_naive())
    }

    /// Within-day clock time of each label, matching
    /// `pd.DatetimeIndex.time`.
    #[must_use]
    pub fn time(&self) -> Vec<Option<chrono::NaiveTime>> {
        map_datetime_labels(self.index.labels(), |dt| dt.time())
    }

    /// Time component preserving timezone semantics, matching
    /// `pd.DatetimeIndex.timetz`. FrankenPandas currently stores
    /// timezone-naive UTC nanoseconds, so this matches [`Self::time`].
    #[must_use]
    pub fn timetz(&self) -> Vec<Option<chrono::NaiveTime>> {
        self.time()
    }

    /// Convert each label to its Julian Date, matching
    /// `pd.DatetimeIndex.to_julian_date()`. The formula is
    /// `JD = unix_seconds / 86400 + 2440587.5` and the result is
    /// computed in f64; NAT propagates as `None`.
    #[must_use]
    pub fn to_julian_date(&self) -> Vec<Option<f64>> {
        const SECONDS_PER_DAY: f64 = 86_400.0;
        const UNIX_EPOCH_JD: f64 = 2_440_587.5;
        self.index
            .labels()
            .iter()
            .map(|label| match label {
                IndexLabel::Datetime64(nanos) if *nanos != i64::MIN => {
                    let secs = (*nanos as f64) / 1_000_000_000.0;
                    Some(secs / SECONDS_PER_DAY + UNIX_EPOCH_JD)
                }
                _ => None,
            })
            .collect()
    }

    /// Annotate a tz-naive index with `tz`, matching
    /// `pd.DatetimeIndex.tz_localize(tz)`. FrankenPandas's storage is
    /// already UTC-naive so localizing to `"UTC"` is a no-op clone;
    /// every other timezone rejects until full tz metadata lands.
    pub fn tz_localize(&self, tz: &str) -> Result<Self, IndexError> {
        match tz {
            "UTC" | "utc" => Ok(self.clone()),
            other => Err(IndexError::InvalidArgument(format!(
                "tz_localize: only 'UTC' is supported until timezone metadata lands; got {other:?}"
            ))),
        }
    }

    /// Convert a tz-aware index from its current zone to `tz`, matching
    /// `pd.DatetimeIndex.tz_convert(tz)`. FrankenPandas indexes are
    /// tz-naive (no source timezone) so this always rejects.
    pub fn tz_convert(&self, _tz: &str) -> Result<Self, IndexError> {
        Err(IndexError::InvalidArgument(
            "tz_convert: cannot convert tz-naive timestamps; call tz_localize('UTC') first"
                .to_owned(),
        ))
    }

    /// Timezone label, matching `pd.DatetimeIndex.tz`. FrankenPandas
    /// stores naive UTC nanos so this always returns `None`; a
    /// follow-up bead will introduce timezone metadata.
    #[must_use]
    pub fn tz(&self) -> Option<String> {
        None
    }

    /// Alias for [`tz`], matching `pd.DatetimeIndex.tzinfo`.
    #[must_use]
    pub fn tzinfo(&self) -> Option<String> {
        self.tz()
    }

    /// Frequency string, matching `pd.DatetimeIndex.freq`. FrankenPandas
    /// does not infer datetime frequency yet so this returns `None`.
    #[must_use]
    pub fn freq(&self) -> Option<String> {
        None
    }

    /// Frequency alias string, matching `pd.DatetimeIndex.freqstr`.
    #[must_use]
    pub fn freqstr(&self) -> Option<String> {
        self.freq()
    }

    /// Inferred frequency, matching `pd.DatetimeIndex.inferred_freq`.
    #[must_use]
    pub fn inferred_freq(&self) -> Option<String> {
        None
    }

    /// Cast to a different storage resolution, matching
    /// `pd.DatetimeIndex.as_unit(unit)`. FrankenPandas's storage is fixed
    /// at nanoseconds so only `"ns"` is supported as a no-op clone; other
    /// units reject with a typed compatibility error.
    pub fn as_unit(&self, unit: &str) -> Result<Self, IndexError> {
        match unit {
            "ns" => Ok(self.clone()),
            other => Err(IndexError::InvalidArgument(format!(
                "as_unit: only 'ns' is supported by FrankenPandas's Datetime64 storage; got {other:?}"
            ))),
        }
    }

    /// Storage resolution unit, matching `pd.DatetimeIndex.unit`. Always
    /// `"ns"` because FrankenPandas stores Datetime64 as nanoseconds.
    #[must_use]
    pub fn unit(&self) -> &'static str {
        "ns"
    }

    /// Resolution string, matching `pd.DatetimeIndex.resolution`. Always
    /// `"nanosecond"` because the underlying storage is fixed at ns.
    #[must_use]
    pub fn resolution(&self) -> &'static str {
        "nanosecond"
    }

    /// First position of `value`, matching `pd.DatetimeIndex.get_loc(value)`.
    /// Pandas raises KeyError for missing values; this surface mirrors
    /// that with [`IndexError::InvalidArgument`].
    pub fn get_loc(&self, value: i64) -> Result<usize, IndexError> {
        // Delegate to Index::position, which binary-searches a monotonic
        // (AscendingDatetime64) index in O(log n) instead of the O(n) linear
        // scan, and falls back to the same first-match linear scan when unsorted
        // (br-frankenpandas-idxdup). Bit-identical: a Datetime64(value) needle
        // matches exactly the labels this scan accepted.
        self.index
            .position(&IndexLabel::Datetime64(value))
            .ok_or_else(|| {
                IndexError::InvalidArgument(format!("get_loc: {value} not in DatetimeIndex"))
            })
    }

    /// Set the index name, matching `pd.DatetimeIndex.rename(name)`.
    /// Alias for set_name.
    #[must_use]
    pub fn rename(&self, name: &str) -> Self {
        self.set_name(name)
    }

    /// Reindex against `target`, matching
    /// `pd.DatetimeIndex.reindex(target)`. Returns
    /// `(target.clone(), indexer)` where indexer is the per-target
    /// position from get_indexer (with -1 for missing).
    #[must_use]
    pub fn reindex(&self, target: &Self) -> (Self, Vec<isize>) {
        let labels: Vec<i64> = target
            .index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Datetime64(n) => Some(*n),
                _ => None,
            })
            .collect();
        let indexer = self.get_indexer(&labels);
        (target.clone(), indexer)
    }

    /// Locate every position matching each target, matching
    /// `pd.DatetimeIndex.get_indexer_non_unique(targets)`. Returns
    /// `(positions, missing)` where `positions` lists every source
    /// position matching any target (in target order) and `missing`
    /// lists target ordinals that had no match.
    #[must_use]
    pub fn get_indexer_non_unique(&self, targets: &[i64]) -> (Vec<isize>, Vec<usize>) {
        let labels = self.index.labels();
        let mut by_value = FxHashMap::<i64, Vec<usize>>::default();
        for (i, label) in labels.iter().enumerate() {
            if let IndexLabel::Datetime64(n) = label {
                by_value.entry(*n).or_default().push(i);
            }
        }
        let mut positions = Vec::<isize>::new();
        let mut missing = Vec::<usize>::new();
        for (idx, target) in targets.iter().enumerate() {
            if let Some(matches) = by_value.get(target) {
                positions.extend(
                    matches
                        .iter()
                        .map(|p| isize::try_from(*p).unwrap_or(isize::MAX)),
                );
            } else {
                positions.push(-1);
                missing.push(idx);
            }
        }
        (positions, missing)
    }

    /// Alias for [`get_indexer`], matching
    /// `pd.DatetimeIndex.get_indexer_for(targets)`.
    #[must_use]
    pub fn get_indexer_for(&self, targets: &[i64]) -> Vec<isize> {
        self.get_indexer(targets)
    }

    /// Locate each label in `targets`, matching
    /// `pd.DatetimeIndex.get_indexer(targets)`. Returns `Vec<isize>` where
    /// `-1` means "missing".
    #[must_use]
    pub fn get_indexer(&self, targets: &[i64]) -> Vec<isize> {
        let labels = self.index.labels();
        let mut positions = FxHashMap::<i64, isize>::default();
        for (i, label) in labels.iter().enumerate() {
            if let IndexLabel::Datetime64(n) = label {
                positions
                    .entry(*n)
                    .or_insert_with(|| isize::try_from(i).unwrap_or(isize::MAX));
            }
        }
        targets
            .iter()
            .map(|n| positions.get(n).copied().unwrap_or(-1))
            .collect()
    }

    /// Position of the slice boundary for `label` and `side`, matching
    /// `pd.DatetimeIndex.get_slice_bound(label, side)`. Mirrors
    /// `searchsorted(label, side)`.
    pub fn get_slice_bound(&self, label: i64, side: &str) -> Result<usize, IndexError> {
        self.searchsorted(label, side)
    }

    /// Half-open positional range for a label slice, matching
    /// `pd.DatetimeIndex.slice_indexer(start, end)`. Wraps slice_locs
    /// in a `std::ops::Range<usize>`.
    pub fn slice_indexer(
        &self,
        start: i64,
        end: i64,
    ) -> Result<std::ops::Range<usize>, IndexError> {
        let (left, right) = self.slice_locs(start, end)?;
        Ok(left..right)
    }

    /// Find positions of `[start, end]` for a label slice, matching
    /// `pd.DatetimeIndex.slice_locs(start, end)`. Requires the index to
    /// be monotonically increasing; non-monotonic input rejects.
    pub fn slice_locs(&self, start: i64, end: i64) -> Result<(usize, usize), IndexError> {
        if !self.is_monotonic_increasing() {
            return Err(IndexError::InvalidArgument(
                "slice_locs requires a monotonic increasing DatetimeIndex".to_owned(),
            ));
        }
        let left = self.searchsorted(start, "left")?;
        let right = self.searchsorted(end, "right")?;
        Ok((left, right))
    }

    /// Convert to a flat [`Index`], matching
    /// `pd.DatetimeIndex.to_flat_index()`. Clone-as-Index because the
    /// underlying storage is already a flat Index of Datetime64 labels.
    #[must_use]
    pub fn to_flat_index(&self) -> Index {
        self.index.clone()
    }

    /// String accessor for the flat datetime labels.
    #[must_use]
    pub fn r#str(&self) -> IndexStringAccessor<'_> {
        IndexStringAccessor::owned(self.to_flat_index())
    }

    /// One-column row materialization, matching `pd.DatetimeIndex.to_frame(index=False)`.
    #[must_use]
    pub fn to_frame(&self) -> Vec<Vec<IndexLabel>> {
        self.to_flat_index().to_frame()
    }

    /// Series-shaped materialization using datetime labels as both index and values.
    #[must_use]
    pub fn to_series(&self) -> Vec<(IndexLabel, IndexLabel)> {
        self.to_flat_index().to_series()
    }

    /// Whether any datetime label coerces to true.
    #[must_use]
    pub fn any(&self) -> bool {
        self.to_flat_index().any()
    }

    /// Whether all datetime labels coerce to true.
    #[must_use]
    pub fn all(&self) -> bool {
        self.to_flat_index().all()
    }

    /// Get labels for a level. DatetimeIndex is flat and only accepts level 0.
    pub fn get_level_values(&self, level: usize) -> Result<Index, IndexError> {
        self.to_flat_index().get_level_values(level)
    }

    /// Drop a level. DatetimeIndex is flat, so removing its only level is invalid.
    pub fn droplevel(&self, level: usize) -> Result<Index, IndexError> {
        self.to_flat_index().droplevel(level)
    }

    /// Group equal datetime labels into position buckets.
    #[must_use]
    pub fn groupby(&self) -> HashMap<IndexLabel, Vec<usize>> {
        self.to_flat_index().groupby()
    }

    /// Apply a function to each datetime label, returning a flat Index.
    #[must_use]
    pub fn map<F>(&self, func: F) -> Index
    where
        F: Fn(&IndexLabel) -> IndexLabel,
    {
        self.to_flat_index().map(func)
    }

    /// Cast datetime labels to a pandas dtype string, returning a flat Index.
    pub fn astype(&self, dtype: &str) -> Result<Index, IndexError> {
        self.to_flat_index().astype(dtype)
    }

    /// Nearest preceding-or-equal datetime label lookup.
    #[must_use]
    pub fn asof(&self, key: &IndexLabel) -> Option<IndexLabel> {
        self.to_flat_index().asof(key)
    }

    /// Locate nearest preceding-or-equal datetime positions for each target label.
    #[must_use]
    pub fn asof_locs(&self, where_index: &Index, mask: Option<&[bool]>) -> Vec<Option<usize>> {
        self.to_flat_index().asof_locs(where_index, mask)
    }

    /// Drop datetime labels, returning a flat Index.
    #[must_use]
    pub fn drop(&self, labels_to_drop: &[IndexLabel]) -> Index {
        self.to_flat_index().drop(labels_to_drop)
    }

    /// Join datetime labels with another flat Index.
    pub fn join(&self, other: &Index, how: &str) -> Result<Index, IndexError> {
        self.to_flat_index().join(other, how)
    }

    /// Sort datetime labels and return the positional sorter.
    #[must_use]
    pub fn sortlevel(&self) -> (Index, Vec<usize>) {
        self.to_flat_index().sortlevel()
    }

    /// Returns a clone, matching `pd.DatetimeIndex.view()`. FrankenPandas
    /// owns its label storage so view materializes a fresh clone instead
    /// of an aliasing reference.
    #[must_use]
    pub fn view(&self) -> Self {
        self.clone()
    }

    /// Identity transpose for a 1D index, matching
    /// `pd.DatetimeIndex.transpose()`.
    #[must_use]
    pub fn transpose(&self) -> Self {
        self.clone()
    }

    /// Alias for `transpose`, matching `pd.DatetimeIndex.T`.
    #[allow(non_snake_case)]
    #[must_use]
    pub fn T(&self) -> Self {
        self.transpose()
    }

    /// Flatten labels to nanoseconds-since-epoch with NAT preserved,
    /// matching `pd.DatetimeIndex.ravel()`.
    #[must_use]
    pub fn ravel(&self) -> Vec<Option<i64>> {
        self.values()
    }

    /// Number of levels in this Index, matching `pd.DatetimeIndex.nlevels`.
    /// Always `1` because DatetimeIndex is a single-level index.
    #[must_use]
    pub fn nlevels(&self) -> usize {
        1
    }

    /// Identity dtype-reinference for typed indexes, matching
    /// `pd.DatetimeIndex.infer_objects()`.
    #[must_use]
    pub fn infer_objects(&self) -> Self {
        self.clone()
    }

    /// Drop NAT labels, matching `pd.DatetimeIndex.dropna()`. Non-datetime
    /// labels (which the wrapper rejects on construction) and `i64::MIN`
    /// sentinels are removed; surviving labels keep their order.
    pub fn dropna(&self) -> Self {
        let surviving: Vec<i64> = self
            .index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Datetime64(nanos) if *nanos != i64::MIN => Some(*nanos),
                _ => None,
            })
            .collect();
        let mut filtered = Self::new(surviving);
        if let Some(name) = self.name() {
            filtered = filtered.set_name(name);
        }
        filtered
    }

    #[must_use]
    pub fn year(&self) -> Vec<Option<i32>> {
        use chrono::Datelike;
        map_datetime_labels(self.index.labels(), |dt| dt.year())
    }

    #[must_use]
    pub fn month(&self) -> Vec<Option<u32>> {
        use chrono::Datelike;
        map_datetime_labels(self.index.labels(), |dt| dt.month())
    }

    #[must_use]
    pub fn day(&self) -> Vec<Option<u32>> {
        use chrono::Datelike;
        map_datetime_labels(self.index.labels(), |dt| dt.day())
    }

    /// Hour of day per label (0..=23), matching `pd.DatetimeIndex.hour`.
    #[must_use]
    pub fn hour(&self) -> Vec<Option<u32>> {
        use chrono::Timelike;
        map_datetime_labels(self.index.labels(), |dt| dt.hour())
    }

    /// Minute of hour per label (0..=59), matching `pd.DatetimeIndex.minute`.
    #[must_use]
    pub fn minute(&self) -> Vec<Option<u32>> {
        use chrono::Timelike;
        map_datetime_labels(self.index.labels(), |dt| dt.minute())
    }

    /// Second of minute per label (0..=59), matching `pd.DatetimeIndex.second`.
    #[must_use]
    pub fn second(&self) -> Vec<Option<u32>> {
        use chrono::Timelike;
        map_datetime_labels(self.index.labels(), |dt| dt.second())
    }

    /// Microsecond component (0..=999_999), matching `pd.DatetimeIndex.microsecond`.
    /// Computed from the within-second nanosecond bucket: `nanos / 1_000`.
    #[must_use]
    pub fn microsecond(&self) -> Vec<Option<u32>> {
        use chrono::Timelike;
        map_datetime_labels(self.index.labels(), |dt| dt.nanosecond() / 1_000)
    }

    /// Nanosecond component (0..=999), matching `pd.DatetimeIndex.nanosecond`.
    /// Computed from the within-second nanosecond bucket: `nanos % 1_000`.
    #[must_use]
    pub fn nanosecond(&self) -> Vec<Option<u32>> {
        use chrono::Timelike;
        map_datetime_labels(self.index.labels(), |dt| dt.nanosecond() % 1_000)
    }

    /// Integer positions whose clock time equals `time`, matching
    /// `pd.DatetimeIndex.indexer_at_time(time)`.
    pub fn indexer_at_time(&self, time: &str) -> Result<Vec<usize>, IndexError> {
        let target = parse_time_of_day_nanos(time, "DatetimeIndex.indexer_at_time")?;
        Ok(self
            .index
            .labels()
            .iter()
            .enumerate()
            .filter_map(|(position, label)| {
                (datetime_label_time_nanos(label) == Some(target)).then_some(position)
            })
            .collect())
    }

    /// Integer positions whose clock time falls between `start_time` and
    /// `end_time`, matching `pd.DatetimeIndex.indexer_between_time`.
    /// Ranges that cross midnight use pandas' wrap-around semantics.
    pub fn indexer_between_time(
        &self,
        start_time: &str,
        end_time: &str,
        include_start: bool,
        include_end: bool,
    ) -> Result<Vec<usize>, IndexError> {
        let start =
            parse_time_of_day_nanos(start_time, "DatetimeIndex.indexer_between_time start_time")?;
        let end = parse_time_of_day_nanos(end_time, "DatetimeIndex.indexer_between_time end_time")?;
        Ok(self
            .index
            .labels()
            .iter()
            .enumerate()
            .filter_map(|(position, label)| {
                datetime_label_time_nanos(label)
                    .filter(|time| {
                        time_nanos_in_between(*time, start, end, include_start, include_end)
                    })
                    .map(|_| position)
            })
            .collect())
    }

    /// ISO 8601 week-of-year (1..=53), matching `pd.DatetimeIndex.week`
    /// (a deprecated pandas alias preserved for parity).
    #[must_use]
    pub fn week(&self) -> Vec<Option<u32>> {
        use chrono::Datelike;
        map_datetime_labels(self.index.labels(), |dt| dt.iso_week().week())
    }

    /// ISO calendar `(year, week, weekday)` triples, matching
    /// `pd.DatetimeIndex.isocalendar()`. Weekday uses pandas' Monday=1
    /// through Sunday=7 convention.
    #[must_use]
    pub fn isocalendar(&self) -> Vec<Option<(i32, u32, u32)>> {
        use chrono::Datelike;
        map_datetime_labels(self.index.labels(), |dt| {
            let iso = dt.iso_week();
            (iso.year(), iso.week(), dt.weekday().number_from_monday())
        })
    }

    /// Alias for [`week`], matching `pd.DatetimeIndex.weekofyear`.
    #[must_use]
    pub fn weekofyear(&self) -> Vec<Option<u32>> {
        self.week()
    }

    /// Day of year (1..=366), matching `pd.DatetimeIndex.dayofyear`.
    #[must_use]
    pub fn dayofyear(&self) -> Vec<Option<u32>> {
        use chrono::Datelike;
        map_datetime_labels(self.index.labels(), |dt| dt.ordinal())
    }

    /// Alias for [`dayofyear`], matching `pd.DatetimeIndex.day_of_year`.
    #[must_use]
    pub fn day_of_year(&self) -> Vec<Option<u32>> {
        self.dayofyear()
    }

    /// Weekday number (Monday=0..Sunday=6), matching
    /// `pd.DatetimeIndex.dayofweek`.
    #[must_use]
    pub fn dayofweek(&self) -> Vec<Option<u32>> {
        use chrono::Datelike;
        map_datetime_labels(self.index.labels(), |dt| {
            dt.weekday().num_days_from_monday()
        })
    }

    /// Alias for [`dayofweek`], matching `pd.DatetimeIndex.day_of_week`.
    #[must_use]
    pub fn day_of_week(&self) -> Vec<Option<u32>> {
        self.dayofweek()
    }

    /// Alias for [`dayofweek`], matching `pd.DatetimeIndex.weekday`.
    #[must_use]
    pub fn weekday(&self) -> Vec<Option<u32>> {
        self.dayofweek()
    }

    /// Calendar quarter (1..=4), matching `pd.DatetimeIndex.quarter`.
    #[must_use]
    pub fn quarter(&self) -> Vec<Option<u32>> {
        use chrono::Datelike;
        map_datetime_labels(self.index.labels(), |dt| (dt.month() - 1) / 3 + 1)
    }

    /// Whether the year is a leap year, matching
    /// `pd.DatetimeIndex.is_leap_year`.
    #[must_use]
    pub fn is_leap_year(&self) -> Vec<Option<bool>> {
        use chrono::Datelike;
        map_datetime_labels(self.index.labels(), |dt| {
            chrono::NaiveDate::from_ymd_opt(dt.year(), 1, 1).is_some_and(|d| d.leap_year())
        })
    }

    /// Number of days in the calendar month of each label,
    /// matching `pd.DatetimeIndex.days_in_month`.
    #[must_use]
    pub fn days_in_month(&self) -> Vec<Option<u32>> {
        use chrono::Datelike;
        map_datetime_labels(self.index.labels(), |dt| {
            days_in_calendar_month(dt.year(), dt.month())
        })
    }

    /// Alias for [`days_in_month`], matching `pd.DatetimeIndex.daysinmonth`.
    #[must_use]
    pub fn daysinmonth(&self) -> Vec<Option<u32>> {
        self.days_in_month()
    }

    /// Whether the day is the first of the month, matching
    /// `pd.DatetimeIndex.is_month_start`.
    #[must_use]
    pub fn is_month_start(&self) -> Vec<Option<bool>> {
        use chrono::Datelike;
        map_datetime_labels(self.index.labels(), |dt| dt.day() == 1)
    }

    /// Whether the day is the last of the month, matching
    /// `pd.DatetimeIndex.is_month_end`.
    #[must_use]
    pub fn is_month_end(&self) -> Vec<Option<bool>> {
        use chrono::Datelike;
        map_datetime_labels(self.index.labels(), |dt| {
            dt.day() == days_in_calendar_month(dt.year(), dt.month())
        })
    }

    /// Whether the timestamp is the first day of a quarter, matching
    /// `pd.DatetimeIndex.is_quarter_start`. Quarter starts: Jan/Apr/Jul/Oct day 1.
    #[must_use]
    pub fn is_quarter_start(&self) -> Vec<Option<bool>> {
        use chrono::Datelike;
        map_datetime_labels(self.index.labels(), |dt| {
            matches!(dt.month(), 1 | 4 | 7 | 10) && dt.day() == 1
        })
    }

    /// Whether the timestamp is the last day of a quarter, matching
    /// `pd.DatetimeIndex.is_quarter_end`. Quarter ends: Mar/Jun/Sep/Dec last day.
    #[must_use]
    pub fn is_quarter_end(&self) -> Vec<Option<bool>> {
        use chrono::Datelike;
        map_datetime_labels(self.index.labels(), |dt| {
            matches!(dt.month(), 3 | 6 | 9 | 12)
                && dt.day() == days_in_calendar_month(dt.year(), dt.month())
        })
    }

    /// Whether the timestamp is January 1, matching
    /// `pd.DatetimeIndex.is_year_start`.
    #[must_use]
    pub fn is_year_start(&self) -> Vec<Option<bool>> {
        use chrono::Datelike;
        map_datetime_labels(self.index.labels(), |dt| dt.month() == 1 && dt.day() == 1)
    }

    /// Whether the timestamp is December 31, matching
    /// `pd.DatetimeIndex.is_year_end`.
    #[must_use]
    pub fn is_year_end(&self) -> Vec<Option<bool>> {
        use chrono::Datelike;
        map_datetime_labels(self.index.labels(), |dt| dt.month() == 12 && dt.day() == 31)
    }

    /// Full English month name, matching `pd.DatetimeIndex.month_name()`.
    #[must_use]
    pub fn month_name(&self) -> Vec<Option<String>> {
        use chrono::Datelike;
        map_datetime_labels(self.index.labels(), |dt| {
            month_name_english(dt.month()).to_owned()
        })
    }

    /// Full English weekday name, matching `pd.DatetimeIndex.day_name()`.
    #[must_use]
    pub fn day_name(&self) -> Vec<Option<String>> {
        use chrono::Datelike;
        map_datetime_labels(self.index.labels(), |dt| {
            weekday_name_english(dt.weekday()).to_owned()
        })
    }

    /// Truncate every timestamp to midnight UTC, matching
    /// `pd.DatetimeIndex.normalize()`. NAT labels propagate.
    #[must_use]
    pub fn normalize(&self) -> Self {
        let nanos: Vec<i64> = self
            .index
            .labels()
            .iter()
            .map(|label| match label {
                IndexLabel::Datetime64(nanos) if *nanos != i64::MIN => {
                    let secs_per_day: i64 = 86_400;
                    let nanos_per_day: i64 = secs_per_day * 1_000_000_000;
                    nanos.div_euclid(nanos_per_day) * nanos_per_day
                }
                _ => i64::MIN,
            })
            .collect();
        let mut normalized = Self::new(nanos);
        if let Some(name) = self.name() {
            normalized = normalized.set_name(name);
        }
        normalized
    }

    /// Whether every label is at midnight UTC (NAT counts as normalized),
    /// matching `pd.DatetimeIndex.is_normalized`.
    #[must_use]
    pub fn is_normalized(&self) -> bool {
        let nanos_per_day: i64 = 86_400 * 1_000_000_000;
        self.index.labels().iter().all(|label| match label {
            IndexLabel::Datetime64(nanos) => {
                *nanos == i64::MIN || nanos.rem_euclid(nanos_per_day) == 0
            }
            _ => true,
        })
    }
}

fn month_name_english(month: u32) -> &'static str {
    match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "",
    }
}

fn weekday_name_english(weekday: chrono::Weekday) -> &'static str {
    match weekday {
        chrono::Weekday::Mon => "Monday",
        chrono::Weekday::Tue => "Tuesday",
        chrono::Weekday::Wed => "Wednesday",
        chrono::Weekday::Thu => "Thursday",
        chrono::Weekday::Fri => "Friday",
        chrono::Weekday::Sat => "Saturday",
        chrono::Weekday::Sun => "Sunday",
    }
}

fn days_in_calendar_month(year: i32, month: u32) -> u32 {
    let next_month = if month == 12 { 1 } else { month + 1 };
    let next_year = if month == 12 { year + 1 } else { year };
    let first_of_next = chrono::NaiveDate::from_ymd_opt(next_year, next_month, 1);
    let first_of_this = chrono::NaiveDate::from_ymd_opt(year, month, 1);
    match (first_of_next, first_of_this) {
        (Some(next), Some(this)) => (next - this).num_days() as u32,
        _ => 0,
    }
}

/// Public pandas-style timedelta index wrapper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimedeltaIndex {
    index: Index,
}

impl TimedeltaIndex {
    #[must_use]
    pub fn new(nanos: Vec<i64>) -> Self {
        Self {
            index: Index::from_timedelta64(nanos),
        }
    }

    pub fn from_index(index: Index) -> Result<Self, IndexError> {
        ensure_index_kind(
            &index,
            |label| matches!(label, IndexLabel::Timedelta64(_)),
            "TimedeltaIndex",
        )?;
        Ok(Self { index })
    }

    #[must_use]
    pub fn as_index(&self) -> &Index {
        &self.index
    }

    #[must_use]
    pub fn into_index(self) -> Index {
        self.index
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.index.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.index.name()
    }

    #[must_use]
    pub fn set_name(&self, name: &str) -> Self {
        Self {
            index: self.index.set_name(name),
        }
    }

    #[must_use]
    pub fn set_names(&self, name: Option<&str>) -> Self {
        Self {
            index: self.index.set_names(name),
        }
    }

    #[must_use]
    pub fn rename_index(&self, name: Option<&str>) -> Self {
        self.set_names(name)
    }

    #[must_use]
    pub fn names(&self) -> Vec<Option<String>> {
        self.index.names()
    }

    #[must_use]
    pub fn copy(&self) -> Self {
        self.clone()
    }

    #[must_use]
    pub fn shape(&self) -> (usize,) {
        self.index.shape()
    }

    #[must_use]
    pub fn size(&self) -> usize {
        self.index.size()
    }

    #[must_use]
    pub fn empty(&self) -> bool {
        self.index.empty()
    }

    #[must_use]
    pub fn dtype(&self) -> &'static str {
        "timedelta64[ns]"
    }

    #[must_use]
    pub fn dtypes(&self) -> Vec<&'static str> {
        vec![self.dtype()]
    }

    #[must_use]
    pub fn memory_usage(&self, deep: bool) -> usize {
        self.index.memory_usage(deep)
    }

    #[must_use]
    pub fn nbytes(&self) -> usize {
        self.index.nbytes()
    }

    #[must_use]
    pub fn hasnans(&self) -> bool {
        self.index.hasnans()
    }

    #[must_use]
    pub fn isna(&self) -> Vec<bool> {
        self.index.isna()
    }

    #[must_use]
    pub fn notna(&self) -> Vec<bool> {
        self.index.notna()
    }

    #[must_use]
    pub fn is_unique(&self) -> bool {
        self.index.is_unique()
    }

    #[must_use]
    pub fn has_duplicates(&self) -> bool {
        self.index.has_duplicates()
    }

    #[must_use]
    pub fn is_monotonic_increasing(&self) -> bool {
        self.index.is_monotonic_increasing()
    }

    #[must_use]
    pub fn is_monotonic(&self) -> bool {
        self.index.is_monotonic()
    }

    #[must_use]
    pub fn is_monotonic_decreasing(&self) -> bool {
        self.index.is_monotonic_decreasing()
    }

    #[must_use]
    pub fn nunique(&self) -> usize {
        self.index.nunique()
    }

    #[must_use]
    pub fn nunique_with_dropna(&self, dropna: bool) -> usize {
        self.index.nunique_with_dropna(dropna)
    }

    #[must_use]
    pub fn ndim(&self) -> usize {
        self.index.ndim()
    }

    pub fn item(&self) -> Result<Option<i64>, IndexError> {
        match self.index.item()? {
            IndexLabel::Timedelta64(nanos) if nanos != Timedelta::NAT => Ok(Some(nanos)),
            IndexLabel::Timedelta64(_) => Ok(None),
            label => Err(IndexError::InvalidArgument(format!(
                "TimedeltaIndex item must be timedelta64, got {label}"
            ))),
        }
    }

    #[must_use]
    pub fn is_(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }

    #[must_use]
    pub fn equals(&self, other: &Self) -> bool {
        self.index.equals(&other.index)
    }

    #[must_use]
    pub fn identical(&self, other: &Self) -> bool {
        self.index.identical(&other.index)
    }

    #[must_use]
    pub fn holds_integer(&self) -> bool {
        false
    }

    #[must_use]
    pub fn inferred_type(&self) -> &'static str {
        "timedelta64"
    }

    #[must_use]
    pub fn is_boolean(&self) -> bool {
        false
    }

    #[must_use]
    pub fn is_categorical(&self) -> bool {
        false
    }

    #[must_use]
    pub fn is_floating(&self) -> bool {
        false
    }

    #[must_use]
    pub fn is_integer(&self) -> bool {
        false
    }

    #[must_use]
    pub fn is_interval(&self) -> bool {
        false
    }

    #[must_use]
    pub fn is_numeric(&self) -> bool {
        false
    }

    #[must_use]
    pub fn is_object(&self) -> bool {
        false
    }

    #[must_use]
    pub fn nanos(&self) -> Vec<Option<i64>> {
        map_timedelta_labels(self.index.labels(), |nanos| nanos)
    }

    #[must_use]
    pub fn values(&self) -> Vec<Option<i64>> {
        self.nanos()
    }

    #[must_use]
    pub fn to_list(&self) -> Vec<Option<i64>> {
        self.nanos()
    }

    #[must_use]
    pub fn tolist(&self) -> Vec<Option<i64>> {
        self.to_list()
    }

    #[must_use]
    pub fn to_numpy(&self) -> Vec<Option<i64>> {
        self.nanos()
    }

    #[must_use]
    pub fn array(&self) -> Vec<Option<i64>> {
        self.nanos()
    }

    #[must_use]
    pub fn days(&self) -> Vec<Option<i64>> {
        map_timedelta_labels(self.index.labels(), |nanos| {
            nanos.div_euclid(Timedelta::NANOS_PER_DAY)
        })
    }

    #[must_use]
    pub fn seconds(&self) -> Vec<Option<i64>> {
        map_timedelta_labels(self.index.labels(), |nanos| {
            nanos.rem_euclid(Timedelta::NANOS_PER_DAY) / Timedelta::NANOS_PER_SEC
        })
    }

    #[must_use]
    pub fn total_seconds(&self) -> Vec<Option<f64>> {
        map_timedelta_labels(self.index.labels(), Timedelta::total_seconds)
    }

    /// Calendar-style component rows, matching `pd.TimedeltaIndex.components`.
    /// NAT propagates as `None`.
    #[must_use]
    pub fn components(&self) -> Vec<Option<TimedeltaComponents>> {
        map_timedelta_labels(self.index.labels(), timedelta_components_for_index)
    }

    /// Underlying nanosecond duration, matching `pd.TimedeltaIndex.asi8`.
    /// `Timedelta::NAT` is preserved at the sentinel value.
    #[must_use]
    pub fn asi8(&self) -> Vec<i64> {
        self.index
            .labels()
            .iter()
            .map(|label| match label {
                IndexLabel::Timedelta64(nanos) => *nanos,
                IndexLabel::Int64(_)
                | IndexLabel::Float64(_)
                | IndexLabel::Bool(_)
                | IndexLabel::Utf8(_)
                | IndexLabel::Datetime64(_)
                | IndexLabel::Null(_) => Timedelta::NAT,
            })
            .collect()
    }

    /// Microseconds-within-second component (0..=999_999), matching
    /// `pd.TimedeltaIndex.microseconds`.
    #[must_use]
    pub fn microseconds(&self) -> Vec<Option<i64>> {
        map_timedelta_labels(self.index.labels(), |nanos| {
            nanos.rem_euclid(Timedelta::NANOS_PER_SEC) / 1_000
        })
    }

    /// Nanoseconds-within-microsecond component (0..=999), matching
    /// `pd.TimedeltaIndex.nanoseconds`.
    #[must_use]
    pub fn nanoseconds(&self) -> Vec<Option<i64>> {
        map_timedelta_labels(self.index.labels(), |nanos| nanos.rem_euclid(1_000))
    }

    /// Position of the maximum label, matching `pd.TimedeltaIndex.argmax()`.
    /// Skips NAT and returns the first-tied position to match pandas
    /// `skipna=True` default.
    pub fn argmax(&self) -> Result<usize, IndexError> {
        let labels = self.index.labels();
        let mut best: Option<usize> = None;
        for (i, label) in labels.iter().enumerate() {
            let nanos = match label {
                IndexLabel::Timedelta64(n) if *n != Timedelta::NAT => *n,
                _ => continue,
            };
            best = Some(match best {
                Some(b) => match labels[b] {
                    IndexLabel::Timedelta64(prev) if nanos > prev => i,
                    _ => b,
                },
                None => i,
            });
        }
        best.ok_or_else(|| {
            IndexError::InvalidArgument("attempt to get argmax of an empty sequence".to_owned())
        })
    }

    /// Position of the minimum label, matching `pd.TimedeltaIndex.argmin()`.
    pub fn argmin(&self) -> Result<usize, IndexError> {
        let labels = self.index.labels();
        let mut best: Option<usize> = None;
        for (i, label) in labels.iter().enumerate() {
            let nanos = match label {
                IndexLabel::Timedelta64(n) if *n != Timedelta::NAT => *n,
                _ => continue,
            };
            best = Some(match best {
                Some(b) => match labels[b] {
                    IndexLabel::Timedelta64(prev) if nanos < prev => i,
                    _ => b,
                },
                None => i,
            });
        }
        best.ok_or_else(|| {
            IndexError::InvalidArgument("attempt to get argmin of an empty sequence".to_owned())
        })
    }

    /// Positions that would sort the labels ascending, matching
    /// `pd.TimedeltaIndex.argsort()`.
    #[must_use]
    pub fn argsort(&self) -> Vec<usize> {
        self.index.argsort()
    }

    /// First-seen unique labels, matching `pd.TimedeltaIndex.unique()`.
    pub fn unique(&self) -> Result<Self, IndexError> {
        Self::from_index(self.index.unique())
    }

    /// Factorization, matching `pd.TimedeltaIndex.factorize()`. NAT inputs
    /// receive `-1` codes; uniques excludes NAT.
    pub fn factorize(&self) -> Result<(Vec<isize>, Self), IndexError> {
        let (codes, uniques) = self.index.factorize();
        Ok((codes, Self::from_index(uniques)?))
    }

    /// Value counts, matching `pd.TimedeltaIndex.value_counts()`. NAT is
    /// dropped by default to match pandas.
    #[must_use]
    pub fn value_counts(&self) -> Vec<(IndexLabel, usize)> {
        self.index.value_counts()
    }

    /// Duplicate mask per position, matching
    /// `pd.TimedeltaIndex.duplicated(keep)`.
    #[must_use]
    pub fn duplicated(&self, keep: DuplicateKeep) -> Vec<bool> {
        self.index.duplicated(keep)
    }

    /// Drop duplicate labels, matching `pd.TimedeltaIndex.drop_duplicates()`.
    pub fn drop_duplicates(&self) -> Result<Self, IndexError> {
        Self::from_index(self.index.drop_duplicates())
    }

    /// Replace positions where `cond` is `false` with `other`, matching
    /// `pd.TimedeltaIndex.where(cond, other)`. Pass `Timedelta::NAT` to
    /// insert NAT.
    pub fn r#where(&self, cond: &[bool], other: i64) -> Result<Self, IndexError> {
        let labels = self.index.labels();
        if cond.len() != labels.len() {
            return Err(IndexError::LengthMismatch {
                expected: labels.len(),
                actual: cond.len(),
                context: "where: cond length must match index length".to_owned(),
            });
        }
        let nanos: Vec<i64> = labels
            .iter()
            .zip(cond.iter())
            .map(|(label, &keep)| {
                if keep {
                    match label {
                        IndexLabel::Timedelta64(n) => *n,
                        _ => Timedelta::NAT,
                    }
                } else {
                    other
                }
            })
            .collect();
        let mut out = Self::new(nanos);
        if let Some(name) = self.name() {
            out = out.set_name(name);
        }
        Ok(out)
    }

    /// Replace positions where `mask` is `true` with `value`, matching
    /// `pd.TimedeltaIndex.putmask(mask, value)`.
    pub fn putmask(&self, mask: &[bool], value: i64) -> Result<Self, IndexError> {
        let labels = self.index.labels();
        if mask.len() != labels.len() {
            return Err(IndexError::LengthMismatch {
                expected: labels.len(),
                actual: mask.len(),
                context: "putmask: mask length must match index length".to_owned(),
            });
        }
        let nanos: Vec<i64> = labels
            .iter()
            .zip(mask.iter())
            .map(|(label, &replace)| {
                if replace {
                    value
                } else {
                    match label {
                        IndexLabel::Timedelta64(n) => *n,
                        _ => Timedelta::NAT,
                    }
                }
            })
            .collect();
        let mut out = Self::new(nanos);
        if let Some(name) = self.name() {
            out = out.set_name(name);
        }
        Ok(out)
    }

    /// Binary-search insertion position, matching
    /// `pd.TimedeltaIndex.searchsorted(value, side)`. The needle is a
    /// nanosecond duration; NAT needles raise.
    pub fn searchsorted(&self, value: i64, side: &str) -> Result<usize, IndexError> {
        self.index
            .searchsorted(&IndexLabel::Timedelta64(value), side)
    }

    /// Insert `value` at position `loc`, matching
    /// `pd.TimedeltaIndex.insert(loc, value)`. `loc == len()` appends;
    /// `loc > len()` raises [`IndexError::OutOfBounds`].
    pub fn insert(&self, loc: usize, value: i64) -> Result<Self, IndexError> {
        let labels = self.index.labels();
        if loc > labels.len() {
            return Err(IndexError::OutOfBounds {
                position: loc,
                length: labels.len(),
            });
        }
        let mut nanos: Vec<i64> = labels
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Timedelta64(n) => Some(*n),
                _ => None,
            })
            .collect();
        nanos.insert(loc, value);
        let mut out = Self::new(nanos);
        if let Some(name) = self.name() {
            out = out.set_name(name);
        }
        Ok(out)
    }

    /// Stringify each label, matching `pd.TimedeltaIndex.format()`.
    /// Non-NAT labels render as a signed nanosecond integer; NAT renders
    /// as the `NaT` literal.
    #[must_use]
    pub fn format(&self) -> Vec<String> {
        self.index
            .labels()
            .iter()
            .map(|label| match label {
                IndexLabel::Timedelta64(nanos) if *nanos != Timedelta::NAT => nanos.to_string(),
                _ => "NaT".to_owned(),
            })
            .collect()
    }

    /// Replace NAT positions with `value`, matching
    /// `pd.TimedeltaIndex.fillna(value)`. Preserves the index name.
    #[must_use]
    pub fn fillna(&self, value: i64) -> Self {
        let nanos: Vec<i64> = self
            .index
            .labels()
            .iter()
            .map(|label| match label {
                IndexLabel::Timedelta64(n) if *n != Timedelta::NAT => *n,
                _ => value,
            })
            .collect();
        let mut out = Self::new(nanos);
        if let Some(name) = self.name() {
            out = out.set_name(name);
        }
        out
    }

    /// Alias for [`isna`], matching `pd.TimedeltaIndex.isnull()`.
    #[must_use]
    pub fn isnull(&self) -> Vec<bool> {
        self.isna()
    }

    /// Alias for [`notna`], matching `pd.TimedeltaIndex.notnull()`.
    #[must_use]
    pub fn notnull(&self) -> Vec<bool> {
        self.notna()
    }

    /// Convert each label to a `chrono::Duration`, matching
    /// `pd.TimedeltaIndex.to_pytimedelta()`. NAT propagates as `None`.
    #[must_use]
    pub fn to_pytimedelta(&self) -> Vec<Option<chrono::Duration>> {
        self.index
            .labels()
            .iter()
            .map(|label| match label {
                IndexLabel::Timedelta64(nanos) if *nanos != Timedelta::NAT => {
                    Some(chrono::Duration::nanoseconds(*nanos))
                }
                _ => None,
            })
            .collect()
    }

    /// Frequency string, matching `pd.TimedeltaIndex.freq`. FrankenPandas
    /// does not infer timedelta frequency yet so this returns `None`.
    #[must_use]
    pub fn freq(&self) -> Option<String> {
        None
    }

    /// Frequency alias string, matching `pd.TimedeltaIndex.freqstr`.
    #[must_use]
    pub fn freqstr(&self) -> Option<String> {
        self.freq()
    }

    /// Inferred frequency, matching `pd.TimedeltaIndex.inferred_freq`.
    #[must_use]
    pub fn inferred_freq(&self) -> Option<String> {
        None
    }

    /// Cast to a different storage resolution, matching
    /// `pd.TimedeltaIndex.as_unit(unit)`. Only `"ns"` is supported.
    pub fn as_unit(&self, unit: &str) -> Result<Self, IndexError> {
        match unit {
            "ns" => Ok(self.clone()),
            other => Err(IndexError::InvalidArgument(format!(
                "as_unit: only 'ns' is supported by FrankenPandas's Timedelta64 storage; got {other:?}"
            ))),
        }
    }

    /// Storage resolution unit, matching `pd.TimedeltaIndex.unit`. Always
    /// `"ns"`.
    #[must_use]
    pub fn unit(&self) -> &'static str {
        "ns"
    }

    /// Resolution string, matching `pd.TimedeltaIndex.resolution`.
    /// Always `"nanosecond"`.
    #[must_use]
    pub fn resolution(&self) -> &'static str {
        "nanosecond"
    }

    /// First position of `value`, matching `pd.TimedeltaIndex.get_loc(value)`.
    pub fn get_loc(&self, value: i64) -> Result<usize, IndexError> {
        // Binary-search a monotonic (AscendingTimedelta64) index via
        // Index::position; same first-match linear fallback when unsorted
        // (br-frankenpandas-idxdup).
        self.index
            .position(&IndexLabel::Timedelta64(value))
            .ok_or_else(|| {
                IndexError::InvalidArgument(format!("get_loc: {value} not in TimedeltaIndex"))
            })
    }

    /// Set the index name, matching `pd.TimedeltaIndex.rename(name)`.
    #[must_use]
    pub fn rename(&self, name: &str) -> Self {
        self.set_name(name)
    }

    /// Reindex against `target`, matching
    /// `pd.TimedeltaIndex.reindex(target)`.
    #[must_use]
    pub fn reindex(&self, target: &Self) -> (Self, Vec<isize>) {
        let labels: Vec<i64> = target
            .index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Timedelta64(n) => Some(*n),
                _ => None,
            })
            .collect();
        let indexer = self.get_indexer(&labels);
        (target.clone(), indexer)
    }

    /// Locate every position matching each target, matching
    /// `pd.TimedeltaIndex.get_indexer_non_unique(targets)`.
    #[must_use]
    pub fn get_indexer_non_unique(&self, targets: &[i64]) -> (Vec<isize>, Vec<usize>) {
        let labels = self.index.labels();
        let mut by_value = FxHashMap::<i64, Vec<usize>>::default();
        for (i, label) in labels.iter().enumerate() {
            if let IndexLabel::Timedelta64(n) = label {
                by_value.entry(*n).or_default().push(i);
            }
        }
        let mut positions = Vec::<isize>::new();
        let mut missing = Vec::<usize>::new();
        for (idx, target) in targets.iter().enumerate() {
            if let Some(matches) = by_value.get(target) {
                positions.extend(
                    matches
                        .iter()
                        .map(|p| isize::try_from(*p).unwrap_or(isize::MAX)),
                );
            } else {
                positions.push(-1);
                missing.push(idx);
            }
        }
        (positions, missing)
    }

    /// Alias for [`get_indexer`], matching
    /// `pd.TimedeltaIndex.get_indexer_for(targets)`.
    #[must_use]
    pub fn get_indexer_for(&self, targets: &[i64]) -> Vec<isize> {
        self.get_indexer(targets)
    }

    /// Locate each label in `targets`, matching
    /// `pd.TimedeltaIndex.get_indexer(targets)`.
    #[must_use]
    pub fn get_indexer(&self, targets: &[i64]) -> Vec<isize> {
        let labels = self.index.labels();
        let mut positions = FxHashMap::<i64, isize>::default();
        for (i, label) in labels.iter().enumerate() {
            if let IndexLabel::Timedelta64(n) = label {
                positions
                    .entry(*n)
                    .or_insert_with(|| isize::try_from(i).unwrap_or(isize::MAX));
            }
        }
        targets
            .iter()
            .map(|n| positions.get(n).copied().unwrap_or(-1))
            .collect()
    }

    /// Position of the slice boundary for `label` and `side`, matching
    /// `pd.TimedeltaIndex.get_slice_bound(label, side)`. Mirrors
    /// `searchsorted(label, side)`.
    pub fn get_slice_bound(&self, label: i64, side: &str) -> Result<usize, IndexError> {
        self.searchsorted(label, side)
    }

    /// Half-open positional range for a label slice, matching
    /// `pd.TimedeltaIndex.slice_indexer(start, end)`.
    pub fn slice_indexer(
        &self,
        start: i64,
        end: i64,
    ) -> Result<std::ops::Range<usize>, IndexError> {
        let (left, right) = self.slice_locs(start, end)?;
        Ok(left..right)
    }

    /// Find positions of `[start, end]` for a label slice, matching
    /// `pd.TimedeltaIndex.slice_locs(start, end)`. Requires the index to
    /// be monotonically increasing.
    pub fn slice_locs(&self, start: i64, end: i64) -> Result<(usize, usize), IndexError> {
        if !self.is_monotonic_increasing() {
            return Err(IndexError::InvalidArgument(
                "slice_locs requires a monotonic increasing TimedeltaIndex".to_owned(),
            ));
        }
        let left = self.searchsorted(start, "left")?;
        let right = self.searchsorted(end, "right")?;
        Ok((left, right))
    }

    /// Convert to a flat [`Index`], matching
    /// `pd.TimedeltaIndex.to_flat_index()`.
    #[must_use]
    pub fn to_flat_index(&self) -> Index {
        self.index.clone()
    }

    /// String accessor for the flat timedelta labels.
    #[must_use]
    pub fn r#str(&self) -> IndexStringAccessor<'_> {
        IndexStringAccessor::owned(self.to_flat_index())
    }

    /// One-column row materialization, matching `pd.TimedeltaIndex.to_frame(index=False)`.
    #[must_use]
    pub fn to_frame(&self) -> Vec<Vec<IndexLabel>> {
        self.to_flat_index().to_frame()
    }

    /// Series-shaped materialization using timedelta labels as both index and values.
    #[must_use]
    pub fn to_series(&self) -> Vec<(IndexLabel, IndexLabel)> {
        self.to_flat_index().to_series()
    }

    /// Whether any timedelta label coerces to true.
    #[must_use]
    pub fn any(&self) -> bool {
        self.to_flat_index().any()
    }

    /// Whether all timedelta labels coerce to true.
    #[must_use]
    pub fn all(&self) -> bool {
        self.to_flat_index().all()
    }

    /// Get labels for a level. TimedeltaIndex is flat and only accepts level 0.
    pub fn get_level_values(&self, level: usize) -> Result<Index, IndexError> {
        self.to_flat_index().get_level_values(level)
    }

    /// Drop a level. TimedeltaIndex is flat, so removing its only level is invalid.
    pub fn droplevel(&self, level: usize) -> Result<Index, IndexError> {
        self.to_flat_index().droplevel(level)
    }

    /// Group equal timedelta labels into position buckets.
    #[must_use]
    pub fn groupby(&self) -> HashMap<IndexLabel, Vec<usize>> {
        self.to_flat_index().groupby()
    }

    /// Apply a function to each timedelta label, returning a flat Index.
    #[must_use]
    pub fn map<F>(&self, func: F) -> Index
    where
        F: Fn(&IndexLabel) -> IndexLabel,
    {
        self.to_flat_index().map(func)
    }

    /// Cast timedelta labels to a pandas dtype string, returning a flat Index.
    pub fn astype(&self, dtype: &str) -> Result<Index, IndexError> {
        self.to_flat_index().astype(dtype)
    }

    /// Nearest preceding-or-equal timedelta label lookup.
    #[must_use]
    pub fn asof(&self, key: &IndexLabel) -> Option<IndexLabel> {
        self.to_flat_index().asof(key)
    }

    /// Locate nearest preceding-or-equal timedelta positions for each target label.
    #[must_use]
    pub fn asof_locs(&self, where_index: &Index, mask: Option<&[bool]>) -> Vec<Option<usize>> {
        self.to_flat_index().asof_locs(where_index, mask)
    }

    /// Drop timedelta labels, returning a flat Index.
    #[must_use]
    pub fn drop(&self, labels_to_drop: &[IndexLabel]) -> Index {
        self.to_flat_index().drop(labels_to_drop)
    }

    /// Join timedelta labels with another flat Index.
    pub fn join(&self, other: &Index, how: &str) -> Result<Index, IndexError> {
        self.to_flat_index().join(other, how)
    }

    /// Sort timedelta labels and return the positional sorter.
    #[must_use]
    pub fn sortlevel(&self) -> (Index, Vec<usize>) {
        self.to_flat_index().sortlevel()
    }

    /// Returns a clone, matching `pd.TimedeltaIndex.view()`.
    #[must_use]
    pub fn view(&self) -> Self {
        self.clone()
    }

    /// Identity transpose for a 1D index, matching
    /// `pd.TimedeltaIndex.transpose()`.
    #[must_use]
    pub fn transpose(&self) -> Self {
        self.clone()
    }

    /// Alias for `transpose`, matching `pd.TimedeltaIndex.T`.
    #[allow(non_snake_case)]
    #[must_use]
    pub fn T(&self) -> Self {
        self.transpose()
    }

    /// Flatten labels to nanosecond durations with NAT preserved,
    /// matching `pd.TimedeltaIndex.ravel()`.
    #[must_use]
    pub fn ravel(&self) -> Vec<Option<i64>> {
        self.values()
    }

    /// Number of levels, matching `pd.TimedeltaIndex.nlevels`. Always `1`.
    #[must_use]
    pub fn nlevels(&self) -> usize {
        1
    }

    /// Identity dtype-reinference for typed indexes, matching
    /// `pd.TimedeltaIndex.infer_objects()`.
    #[must_use]
    pub fn infer_objects(&self) -> Self {
        self.clone()
    }

    /// Drop NAT labels, matching `pd.TimedeltaIndex.dropna()`.
    pub fn dropna(&self) -> Self {
        let surviving: Vec<i64> = self
            .index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Timedelta64(nanos) if *nanos != Timedelta::NAT => Some(*nanos),
                _ => None,
            })
            .collect();
        let mut filtered = Self::new(surviving);
        if let Some(name) = self.name() {
            filtered = filtered.set_name(name);
        }
        filtered
    }

    /// Pick labels at the given positions, matching
    /// `pd.TimedeltaIndex.take()`. Out-of-bounds positions raise
    /// [`IndexError::OutOfBounds`].
    pub fn take(&self, positions: &[usize]) -> Result<Self, IndexError> {
        let labels = self.index.labels();
        for &p in positions {
            if p >= labels.len() {
                return Err(IndexError::OutOfBounds {
                    position: p,
                    length: labels.len(),
                });
            }
        }
        let nanos: Vec<i64> = positions
            .iter()
            .map(|&p| match labels[p] {
                IndexLabel::Timedelta64(n) => n,
                _ => Timedelta::NAT,
            })
            .collect();
        let mut out = Self::new(nanos);
        if let Some(name) = self.name() {
            out = out.set_name(name);
        }
        Ok(out)
    }

    /// Repeat each label `repeats` times, matching
    /// `pd.TimedeltaIndex.repeat()`.
    #[must_use]
    pub fn repeat(&self, repeats: usize) -> Self {
        let mut out = Vec::with_capacity(self.len() * repeats);
        for label in self.index.labels() {
            if let IndexLabel::Timedelta64(n) = label {
                for _ in 0..repeats {
                    out.push(*n);
                }
            }
        }
        let mut result = Self::new(out);
        if let Some(name) = self.name() {
            result = result.set_name(name);
        }
        result
    }

    /// Per-position membership mask, matching
    /// `pd.TimedeltaIndex.isin(values)`. `values` is interpreted as a slice
    /// of nanosecond durations; pass `Timedelta::NAT` to test for NAT.
    #[must_use]
    pub fn isin(&self, values: &[i64]) -> Vec<bool> {
        let needle: FxHashSet<i64> = values.iter().copied().collect();
        self.index
            .labels()
            .iter()
            .map(|label| match label {
                IndexLabel::Timedelta64(n) => needle.contains(n),
                _ => false,
            })
            .collect()
    }

    /// Concatenate with another TimedeltaIndex, matching
    /// `pd.TimedeltaIndex.append(other)`. Preserves the index name when both
    /// operands share it; otherwise pandas drops the name.
    #[must_use]
    pub fn append(&self, other: &Self) -> Self {
        let mut nanos: Vec<i64> = self
            .index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Timedelta64(n) => Some(*n),
                _ => None,
            })
            .collect();
        nanos.extend(other.index.labels().iter().filter_map(|label| match label {
            IndexLabel::Timedelta64(n) => Some(*n),
            _ => None,
        }));
        let mut out = Self::new(nanos);
        if let Some(name) = self.name().filter(|_| self.name() == other.name()) {
            out = out.set_name(name);
        }
        out
    }

    /// Minimum non-NAT label, matching `pd.TimedeltaIndex.min()`.
    #[must_use]
    pub fn min(&self) -> Option<i64> {
        self.index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Timedelta64(n) if *n != Timedelta::NAT => Some(*n),
                _ => None,
            })
            .min()
    }

    /// Shift each label by `periods` units of `freq_nanos`, matching
    /// `pd.TimedeltaIndex.shift(periods, freq)`. NAT propagates as NAT.
    #[must_use]
    pub fn shift(&self, periods: i64, freq_nanos: i64) -> Self {
        let delta = periods.saturating_mul(freq_nanos);
        let nanos: Vec<i64> = self
            .index
            .labels()
            .iter()
            .map(|label| match label {
                IndexLabel::Timedelta64(n) if *n != Timedelta::NAT => n.saturating_add(delta),
                _ => Timedelta::NAT,
            })
            .collect();
        let mut out = Self::new(nanos);
        if let Some(name) = self.name() {
            out = out.set_name(name);
        }
        out
    }

    /// Positional first differences, matching `pd.TimedeltaIndex.diff()`.
    /// NAT inputs propagate and signed `periods` follows pandas' lookup
    /// direction.
    #[must_use]
    pub fn diff(&self, periods: i64) -> Self {
        let labels = self.index.labels();
        optional_diffs_to_timedelta_index(
            positional_diff(labels.len(), periods, |current, previous| {
                match (&labels[current], &labels[previous]) {
                    (
                        IndexLabel::Timedelta64(current_nanos),
                        IndexLabel::Timedelta64(previous_nanos),
                    ) if *current_nanos != Timedelta::NAT && *previous_nanos != Timedelta::NAT => {
                        current_nanos.checked_sub(*previous_nanos)
                    }
                    _ => None,
                }
            }),
            self.name(),
        )
    }

    fn round_fixed_freq(&self, freq: &str, mode: TemporalRoundMode) -> Result<Self, IndexError> {
        let unit_nanos = parse_fixed_temporal_freq(freq, "TimedeltaIndex rounding")?;
        let nanos: Vec<i64> = self
            .index
            .labels()
            .iter()
            .map(|label| match label {
                IndexLabel::Timedelta64(n) if *n != Timedelta::NAT => {
                    round_nanos_to_unit(*n, unit_nanos, mode)
                }
                _ => Timedelta::NAT,
            })
            .collect();
        let mut out = Self::new(nanos);
        if let Some(name) = self.name() {
            out = out.set_name(name);
        }
        Ok(out)
    }

    /// Round timedeltas down to a fixed pandas frequency.
    pub fn floor(&self, freq: &str) -> Result<Self, IndexError> {
        self.round_fixed_freq(freq, TemporalRoundMode::Floor)
    }

    /// Round timedeltas up to a fixed pandas frequency.
    pub fn ceil(&self, freq: &str) -> Result<Self, IndexError> {
        self.round_fixed_freq(freq, TemporalRoundMode::Ceil)
    }

    /// Round timedeltas to the nearest fixed pandas frequency, using half-even ties.
    pub fn round(&self, freq: &str) -> Result<Self, IndexError> {
        self.round_fixed_freq(freq, TemporalRoundMode::Round)
    }

    /// Average non-NAT label as nanosecond duration, matching
    /// `pd.TimedeltaIndex.mean()`. Empty / all-NAT returns `None`.
    #[must_use]
    pub fn mean(&self) -> Option<i64> {
        let mut total: i128 = 0;
        let mut count: i128 = 0;
        for label in self.index.labels() {
            if let IndexLabel::Timedelta64(n) = label
                && *n != Timedelta::NAT
            {
                total += i128::from(*n);
                count += 1;
            }
        }
        if count == 0 {
            return None;
        }
        i64::try_from(total / count).ok()
    }

    /// Sum of non-NAT labels as nanosecond duration, matching
    /// `pd.TimedeltaIndex.sum()`. Returns `Some(0)` for empty inputs to
    /// match pandas. Sum is computed in `i128` to avoid overflow before
    /// narrowing back to `i64`.
    #[must_use]
    pub fn sum(&self) -> Option<i64> {
        let mut total: i128 = 0;
        for label in self.index.labels() {
            if let IndexLabel::Timedelta64(n) = label
                && *n != Timedelta::NAT
            {
                total += i128::from(*n);
            }
        }
        i64::try_from(total).ok()
    }

    /// Sample variance over non-NAT labels in nanoseconds-squared,
    /// matching `pd.TimedeltaIndex.var(ddof=1)`. Returns `None` for
    /// fewer than two non-NAT entries.
    #[must_use]
    pub fn var(&self) -> Option<f64> {
        let nanos: Vec<f64> = self
            .index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Timedelta64(n) if *n != Timedelta::NAT => Some(*n as f64),
                _ => None,
            })
            .collect();
        if nanos.len() < 2 {
            return None;
        }
        let mean = nanos.iter().sum::<f64>() / nanos.len() as f64;
        Some(nanos.iter().map(|n| (n - mean).powi(2)).sum::<f64>() / (nanos.len() as f64 - 1.0))
    }

    /// Sample standard deviation of non-NAT labels in nanoseconds,
    /// matching `pd.TimedeltaIndex.std(ddof=1)`. Returns `None` for
    /// fewer than two non-NAT entries.
    #[must_use]
    pub fn std(&self) -> Option<i64> {
        let nanos: Vec<f64> = self
            .index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Timedelta64(n) if *n != Timedelta::NAT => Some(*n as f64),
                _ => None,
            })
            .collect();
        if nanos.len() < 2 {
            return None;
        }
        let mean = nanos.iter().sum::<f64>() / nanos.len() as f64;
        let var =
            nanos.iter().map(|n| (n - mean).powi(2)).sum::<f64>() / (nanos.len() as f64 - 1.0);
        Some(var.sqrt() as i64)
    }

    /// Median non-NAT label, matching `pd.TimedeltaIndex.median()`.
    #[must_use]
    pub fn median(&self) -> Option<i64> {
        let mut nanos: Vec<i64> = self
            .index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Timedelta64(n) if *n != Timedelta::NAT => Some(*n),
                _ => None,
            })
            .collect();
        if nanos.is_empty() {
            return None;
        }
        nanos.sort_unstable();
        let mid = nanos.len() / 2;
        if nanos.len() % 2 == 1 {
            Some(nanos[mid])
        } else {
            let total = i128::from(nanos[mid - 1]) + i128::from(nanos[mid]);
            i64::try_from(total / 2).ok()
        }
    }

    /// Maximum non-NAT label, matching `pd.TimedeltaIndex.max()`.
    #[must_use]
    pub fn max(&self) -> Option<i64> {
        self.index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Timedelta64(n) if *n != Timedelta::NAT => Some(*n),
                _ => None,
            })
            .max()
    }

    /// Labels present in both indexes, matching
    /// `pd.TimedeltaIndex.intersection(other)`.
    #[must_use]
    pub fn intersection(&self, other: &Self) -> Self {
        let other_set: FxHashSet<i64> = other
            .index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Timedelta64(n) => Some(*n),
                _ => None,
            })
            .collect();
        let mut seen = FxHashSet::<i64>::default();
        let nanos: Vec<i64> = self
            .index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Timedelta64(n) if other_set.contains(n) && seen.insert(*n) => Some(*n),
                _ => None,
            })
            .collect();
        let mut out = Self::new(nanos);
        if let Some(name) = self.name().filter(|_| self.name() == other.name()) {
            out = out.set_name(name);
        }
        out
    }

    /// Labels from self followed by labels from other not already present,
    /// matching `pd.TimedeltaIndex.union(other)`.
    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        let mut seen = FxHashSet::<i64>::default();
        let mut nanos: Vec<i64> = Vec::new();
        for label in self
            .index
            .labels()
            .iter()
            .chain(other.index.labels().iter())
        {
            if let IndexLabel::Timedelta64(n) = label
                && seen.insert(*n)
            {
                nanos.push(*n);
            }
        }
        let mut out = Self::new(nanos);
        if let Some(name) = self.name().filter(|_| self.name() == other.name()) {
            out = out.set_name(name);
        }
        out
    }

    /// Labels in self not in other, matching
    /// `pd.TimedeltaIndex.difference(other)`.
    #[must_use]
    pub fn difference(&self, other: &Self) -> Self {
        let other_set: FxHashSet<i64> = other
            .index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Timedelta64(n) => Some(*n),
                _ => None,
            })
            .collect();
        let mut seen = FxHashSet::<i64>::default();
        let nanos: Vec<i64> = self
            .index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Timedelta64(n) if !other_set.contains(n) && seen.insert(*n) => Some(*n),
                _ => None,
            })
            .collect();
        let mut out = Self::new(nanos);
        // Per br-frankenpandas-6r1lq: difference preserves self.name only
        // (asymmetric op).
        if let Some(name) = self.name() {
            out = out.set_name(name);
        }
        out
    }

    /// Labels in either but not both, matching
    /// `pd.TimedeltaIndex.symmetric_difference(other)`.
    #[must_use]
    pub fn symmetric_difference(&self, other: &Self) -> Self {
        let self_set: FxHashSet<i64> = self
            .index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Timedelta64(n) => Some(*n),
                _ => None,
            })
            .collect();
        let other_set: FxHashSet<i64> = other
            .index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Timedelta64(n) => Some(*n),
                _ => None,
            })
            .collect();
        let mut seen = FxHashSet::<i64>::default();
        let mut nanos: Vec<i64> = Vec::new();
        for label in self.index.labels() {
            if let IndexLabel::Timedelta64(n) = label
                && !other_set.contains(n)
                && seen.insert(*n)
            {
                nanos.push(*n);
            }
        }
        for label in other.index.labels() {
            if let IndexLabel::Timedelta64(n) = label
                && !self_set.contains(n)
                && seen.insert(*n)
            {
                nanos.push(*n);
            }
        }
        let mut out = Self::new(nanos);
        if let Some(name) = self.name().filter(|_| self.name() == other.name()) {
            out = out.set_name(name);
        }
        out
    }

    /// Sort labels ascending, matching `pd.TimedeltaIndex.sort_values()`.
    /// NAT sorts first (Timedelta::NAT sentinel) to match pandas
    /// `na_position='first'` default.
    #[must_use]
    pub fn sort_values(&self) -> Self {
        let mut nanos: Vec<i64> = self
            .index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Timedelta64(n) => Some(*n),
                _ => None,
            })
            .collect();
        nanos.sort_unstable();
        let mut out = Self::new(nanos);
        if let Some(name) = self.name() {
            out = out.set_name(name);
        }
        out
    }

    /// Alias for `sort_values`, matching `pd.TimedeltaIndex.sort()`.
    #[must_use]
    pub fn sort(&self) -> Self {
        self.sort_values()
    }

    /// Remove the label at the given position, matching
    /// `pd.TimedeltaIndex.delete(loc)`.
    pub fn delete(&self, loc: usize) -> Result<Self, IndexError> {
        let labels = self.index.labels();
        if loc >= labels.len() {
            return Err(IndexError::OutOfBounds {
                position: loc,
                length: labels.len(),
            });
        }
        let nanos: Vec<i64> = labels
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != loc)
            .filter_map(|(_, label)| match label {
                IndexLabel::Timedelta64(n) => Some(*n),
                _ => None,
            })
            .collect();
        let mut out = Self::new(nanos);
        if let Some(name) = self.name() {
            out = out.set_name(name);
        }
        Ok(out)
    }
}

/// Public pandas-style period index wrapper.
///
/// `Period` already lives in `fp-types`; this wrapper gives callers a typed
/// index container while DataFrame integration can still materialize through
/// string labels until a dedicated Period `IndexLabel` variant lands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeriodIndex {
    values: Vec<Period>,
    name: Option<String>,
}

impl PeriodIndex {
    #[must_use]
    pub fn new(values: Vec<Period>) -> Self {
        Self { values, name: None }
    }

    /// Construct a PeriodIndex from raw ordinal values and a frequency,
    /// matching `pd.PeriodIndex.from_ordinals(ordinals, freq)`.
    #[must_use]
    pub fn from_ordinals(ordinals: &[i64], freq: PeriodFreq) -> Self {
        let values: Vec<Period> = ordinals
            .iter()
            .map(|&ordinal| Period::new(ordinal, freq))
            .collect();
        Self { values, name: None }
    }

    pub fn from_fields(fields: PeriodFields<'_>) -> Result<Self, IndexError> {
        validate_period_fields(&fields)?;
        let freq = period_fields_freq(&fields)?;
        let values = (0..fields.year.len())
            .map(|position| period_from_fields_at(&fields, freq, position))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { values, name: None })
    }

    #[must_use]
    pub fn from_range(start: Period, periods: usize) -> Self {
        Self::new(fp_types::period_range(start, periods))
    }

    #[must_use]
    pub fn values(&self) -> &[Period] {
        &self.values
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
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    #[must_use]
    pub fn set_name(&self, name: &str) -> Self {
        let mut out = self.clone();
        out.name = Some(name.to_owned());
        out
    }

    #[must_use]
    pub fn set_names(&self, name: Option<&str>) -> Self {
        let mut out = self.clone();
        out.name = name.map(str::to_owned);
        out
    }

    #[must_use]
    pub fn rename_index(&self, name: Option<&str>) -> Self {
        self.set_names(name)
    }

    #[must_use]
    pub fn names(&self) -> Vec<Option<String>> {
        vec![self.name.clone()]
    }

    #[must_use]
    pub fn copy(&self) -> Self {
        self.clone()
    }

    #[must_use]
    pub fn shape(&self) -> (usize,) {
        (self.len(),)
    }

    #[must_use]
    pub fn size(&self) -> usize {
        self.len()
    }

    #[must_use]
    pub fn empty(&self) -> bool {
        self.is_empty()
    }

    #[must_use]
    pub fn dtype(&self) -> String {
        self.freq().map_or_else(
            || "period[unknown]".to_owned(),
            |freq| format!("period[{freq}]"),
        )
    }

    #[must_use]
    pub fn dtypes(&self) -> Vec<String> {
        vec![self.dtype()]
    }

    /// Whether any period label is missing.
    ///
    /// FrankenPandas `Period` currently has no NaT sentinel, so this is
    /// always false until native period missing values are introduced.
    #[must_use]
    pub fn hasnans(&self) -> bool {
        false
    }

    /// Missing-value mask, matching `pd.PeriodIndex.isna()`.
    #[must_use]
    pub fn isna(&self) -> Vec<bool> {
        vec![false; self.len()]
    }

    /// Alias for [`isna`](Self::isna), matching `pd.PeriodIndex.isnull()`.
    #[must_use]
    pub fn isnull(&self) -> Vec<bool> {
        self.isna()
    }

    /// Non-missing mask, matching `pd.PeriodIndex.notna()`.
    #[must_use]
    pub fn notna(&self) -> Vec<bool> {
        vec![true; self.len()]
    }

    /// Alias for [`notna`](Self::notna), matching `pd.PeriodIndex.notnull()`.
    #[must_use]
    pub fn notnull(&self) -> Vec<bool> {
        self.notna()
    }

    /// Drop missing labels, matching `pd.PeriodIndex.dropna()`.
    ///
    /// With no native Period NaT sentinel, this is a name-preserving clone.
    #[must_use]
    pub fn dropna(&self) -> Self {
        self.clone()
    }

    #[must_use]
    pub fn memory_usage(&self, deep: bool) -> usize {
        let name_bytes = if deep {
            self.name.as_ref().map_or(0, String::len)
        } else {
            0
        };
        self.values.len() * std::mem::size_of::<Period>() + name_bytes
    }

    #[must_use]
    pub fn nbytes(&self) -> usize {
        self.memory_usage(false)
    }

    fn compare_periods(left: &Period, right: &Period) -> std::cmp::Ordering {
        left.cmp_same_freq(right).unwrap_or_else(|| {
            left.freq
                .cmp(&right.freq)
                .then(left.ordinal.cmp(&right.ordinal))
        })
    }

    #[must_use]
    pub fn is_unique(&self) -> bool {
        let unique: FxHashSet<&Period> = self.values.iter().collect();
        unique.len() == self.values.len()
    }

    #[must_use]
    pub fn has_duplicates(&self) -> bool {
        !self.is_unique()
    }

    #[must_use]
    pub fn is_monotonic_increasing(&self) -> bool {
        self.values
            .windows(2)
            .all(|window| Self::compare_periods(&window[0], &window[1]).is_le())
    }

    #[must_use]
    pub fn is_monotonic(&self) -> bool {
        self.is_monotonic_increasing()
    }

    #[must_use]
    pub fn is_monotonic_decreasing(&self) -> bool {
        self.values
            .windows(2)
            .all(|window| Self::compare_periods(&window[0], &window[1]).is_ge())
    }

    #[must_use]
    pub fn nunique(&self) -> usize {
        self.values.iter().collect::<FxHashSet<_>>().len()
    }

    #[must_use]
    pub fn ndim(&self) -> usize {
        1
    }

    pub fn item(&self) -> Result<Period, IndexError> {
        if self.values.len() == 1 {
            Ok(self.values[0])
        } else {
            Err(IndexError::InvalidArgument(format!(
                "item requires exactly one label, got {}",
                self.values.len()
            )))
        }
    }

    #[must_use]
    pub fn is_(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }

    #[must_use]
    pub fn equals(&self, other: &Self) -> bool {
        self.values == other.values
    }

    #[must_use]
    pub fn identical(&self, other: &Self) -> bool {
        self.equals(other) && self.name == other.name
    }

    #[must_use]
    pub fn holds_integer(&self) -> bool {
        false
    }

    #[must_use]
    pub fn inferred_type(&self) -> &'static str {
        "period"
    }

    #[must_use]
    pub fn is_boolean(&self) -> bool {
        false
    }

    #[must_use]
    pub fn is_categorical(&self) -> bool {
        false
    }

    #[must_use]
    pub fn is_floating(&self) -> bool {
        false
    }

    #[must_use]
    pub fn is_integer(&self) -> bool {
        false
    }

    #[must_use]
    pub fn is_interval(&self) -> bool {
        false
    }

    #[must_use]
    pub fn is_numeric(&self) -> bool {
        false
    }

    #[must_use]
    pub fn is_object(&self) -> bool {
        false
    }

    #[must_use]
    pub fn freq(&self) -> Option<PeriodFreq> {
        self.values.first().map(|period| period.freq)
    }

    /// Raw period ordinals, matching `pd.PeriodIndex.asi8`.
    #[must_use]
    pub fn asi8(&self) -> Vec<i64> {
        self.values.iter().map(|period| period.ordinal).collect()
    }

    #[must_use]
    pub fn to_list(&self) -> Vec<Period> {
        self.values.clone()
    }

    #[must_use]
    pub fn tolist(&self) -> Vec<Period> {
        self.to_list()
    }

    #[must_use]
    pub fn to_numpy(&self) -> Vec<Period> {
        self.values.clone()
    }

    #[must_use]
    pub fn array(&self) -> Vec<Period> {
        self.values.clone()
    }

    #[must_use]
    pub fn to_index(&self) -> Index {
        Index::from_utf8(self.values.iter().map(Period::to_string).collect())
            .set_names(self.name.as_deref())
    }

    /// First-seen unique periods, matching `pd.PeriodIndex.unique()`.
    /// Preserves the index name.
    #[must_use]
    pub fn unique(&self) -> Self {
        let mut seen = FxHashSet::<&Period>::default();
        let mut uniques = Vec::<Period>::new();
        for period in &self.values {
            if seen.insert(period) {
                uniques.push(*period);
            }
        }
        Self {
            values: uniques,
            name: self.name.clone(),
        }
    }

    /// Per-position duplicate mask, matching
    /// `pd.PeriodIndex.duplicated(keep)`.
    #[must_use]
    pub fn duplicated(&self, keep: DuplicateKeep) -> Vec<bool> {
        let mut result = vec![false; self.values.len()];
        match keep {
            DuplicateKeep::First => {
                let mut seen = FxHashSet::<&Period>::default();
                for (i, period) in self.values.iter().enumerate() {
                    if !seen.insert(period) {
                        result[i] = true;
                    }
                }
            }
            DuplicateKeep::Last => {
                let mut seen = FxHashSet::<&Period>::default();
                for (i, period) in self.values.iter().enumerate().rev() {
                    if !seen.insert(period) {
                        result[i] = true;
                    }
                }
            }
            DuplicateKeep::None => {
                let mut counts = FxHashMap::<&Period, usize>::default();
                for period in &self.values {
                    *counts.entry(period).or_insert(0) += 1;
                }
                for (i, period) in self.values.iter().enumerate() {
                    if counts.get(period).copied().unwrap_or(0) > 1 {
                        result[i] = true;
                    }
                }
            }
        }
        result
    }

    /// Drop duplicate periods (keep first), matching
    /// `pd.PeriodIndex.drop_duplicates()`.
    #[must_use]
    pub fn drop_duplicates(&self) -> Self {
        self.unique()
    }

    /// Value counts, matching `pd.PeriodIndex.value_counts()`. Pandas
    /// sorts descending by count.
    #[must_use]
    pub fn value_counts(&self) -> Vec<(Period, usize)> {
        let mut order = Vec::<&Period>::new();
        let mut counts = FxHashMap::<&Period, usize>::default();
        for period in &self.values {
            let entry = counts.entry(period).or_insert_with(|| {
                order.push(period);
                0
            });
            *entry += 1;
        }
        let mut pairs: Vec<(Period, usize)> = order.iter().map(|p| (**p, counts[*p])).collect();
        pairs.sort_by_key(|entry| std::cmp::Reverse(entry.1));
        pairs
    }

    /// Pick periods at the given positions, matching
    /// `pd.PeriodIndex.take()`. Out-of-bounds positions raise
    /// [`IndexError::OutOfBounds`].
    pub fn take(&self, positions: &[usize]) -> Result<Self, IndexError> {
        for &p in positions {
            if p >= self.values.len() {
                return Err(IndexError::OutOfBounds {
                    position: p,
                    length: self.values.len(),
                });
            }
        }
        let taken: Vec<Period> = positions.iter().map(|&p| self.values[p]).collect();
        Ok(Self {
            values: taken,
            name: self.name.clone(),
        })
    }

    /// Repeat each period `repeats` times, matching
    /// `pd.PeriodIndex.repeat()`.
    #[must_use]
    pub fn repeat(&self, repeats: usize) -> Self {
        let mut out = Vec::with_capacity(self.values.len() * repeats);
        for &period in &self.values {
            for _ in 0..repeats {
                out.push(period);
            }
        }
        Self {
            values: out,
            name: self.name.clone(),
        }
    }

    /// Positional first differences in period-frequency units.
    ///
    /// Pandas returns frequency offset objects for `PeriodIndex.diff()`. The
    /// Rust surface exposes the same semantic payload as ordinal deltas while
    /// preserving null slots for positions without a comparison partner or
    /// mixed-frequency pairs.
    #[must_use]
    pub fn diff(&self, periods: i64) -> Vec<Option<i64>> {
        positional_diff(self.values.len(), periods, |current, previous| {
            self.values[current].diff(&self.values[previous])
        })
    }

    /// Convert periods to a new frequency using pandas' default end boundary.
    ///
    /// Matches `pd.PeriodIndex.asfreq(freq)` for supported target frequencies.
    pub fn asfreq(&self, freq: &str) -> Result<Self, IndexError> {
        self.asfreq_with_how(freq, "end")
    }

    /// Convert periods to a new frequency at the requested boundary.
    ///
    /// Supported `how` values mirror pandas' common aliases:
    /// `start` / `s` / `begin` and `end` / `e` / `finish`.
    pub fn asfreq_with_how(&self, freq: &str, how: &str) -> Result<Self, IndexError> {
        let target_freq = PeriodFreq::parse(freq).ok_or_else(|| {
            IndexError::InvalidArgument(format!("asfreq: unsupported frequency '{freq}'"))
        })?;
        let boundary = parse_period_boundary_how(how, "asfreq")?;
        let values = self
            .values
            .iter()
            .copied()
            .map(|period| {
                let nanos = period_boundary_nanos(period, boundary)?;
                datetime_period_ordinal_at_boundary(nanos, target_freq, boundary)
                    .map(|ordinal| Period::new(ordinal, target_freq))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            values,
            name: self.name.clone(),
        })
    }

    fn to_timestamp_boundary(&self, boundary: PeriodBoundary) -> Result<DatetimeIndex, IndexError> {
        let nanos = self
            .values
            .iter()
            .copied()
            .map(|period| period_boundary_nanos(period, boundary))
            .collect::<Result<Vec<_>, _>>()?;
        let mut out = DatetimeIndex::new(nanos);
        if let Some(name) = self.name() {
            out = out.set_name(name);
        }
        Ok(out)
    }

    /// Timestamp at each period's start boundary.
    ///
    /// Matches `pd.PeriodIndex.start_time`.
    pub fn start_time(&self) -> Result<DatetimeIndex, IndexError> {
        self.to_timestamp_boundary(PeriodBoundary::Start)
    }

    /// Timestamp at each period's inclusive end boundary.
    ///
    /// Matches `pd.PeriodIndex.end_time`.
    pub fn end_time(&self) -> Result<DatetimeIndex, IndexError> {
        self.to_timestamp_boundary(PeriodBoundary::End)
    }

    /// Convert periods to timestamp labels at the requested boundary.
    ///
    /// Supported `how` values mirror pandas' common aliases:
    /// `start` / `s` / `begin` and `end` / `e` / `finish`.
    pub fn to_timestamp(&self, how: &str) -> Result<DatetimeIndex, IndexError> {
        match how.trim().to_ascii_lowercase().as_str() {
            "" | "s" | "start" | "begin" | "b" => self.start_time(),
            "e" | "end" | "finish" => self.end_time(),
            other => Err(IndexError::InvalidArgument(format!(
                "to_timestamp how must be 'start' or 'end', got {other:?}"
            ))),
        }
    }

    /// Fiscal year for each period's ending boundary.
    ///
    /// For the currently supported unanchored frequencies this is the
    /// calendar year of `end_time`, matching pandas' `PeriodIndex.qyear`.
    pub fn qyear(&self) -> Result<Vec<i32>, IndexError> {
        self.values
            .iter()
            .copied()
            .map(period_qyear)
            .collect::<Result<Vec<_>, _>>()
    }

    // ── Per br-frankenpandas-qigpe: date-part accessors (19 methods) ──

    /// Year component for each period, matching `pd.PeriodIndex.year`.
    pub fn year(&self) -> Result<Vec<Option<i32>>, IndexError> {
        Ok(self.start_time()?.year())
    }

    /// Month component (1-12), matching `pd.PeriodIndex.month`.
    pub fn month(&self) -> Result<Vec<Option<u32>>, IndexError> {
        Ok(self.start_time()?.month())
    }

    /// Day of month (1-31), matching `pd.PeriodIndex.day`.
    pub fn day(&self) -> Result<Vec<Option<u32>>, IndexError> {
        Ok(self.start_time()?.day())
    }

    /// Hour (0-23), matching `pd.PeriodIndex.hour`.
    pub fn hour(&self) -> Result<Vec<Option<u32>>, IndexError> {
        Ok(self.start_time()?.hour())
    }

    /// Minute (0-59), matching `pd.PeriodIndex.minute`.
    pub fn minute(&self) -> Result<Vec<Option<u32>>, IndexError> {
        Ok(self.start_time()?.minute())
    }

    /// Second (0-59), matching `pd.PeriodIndex.second`.
    pub fn second(&self) -> Result<Vec<Option<u32>>, IndexError> {
        Ok(self.start_time()?.second())
    }

    /// Quarter (1-4), matching `pd.PeriodIndex.quarter`.
    pub fn quarter(&self) -> Result<Vec<Option<u32>>, IndexError> {
        Ok(self.start_time()?.quarter())
    }

    /// Day of week (0=Monday, 6=Sunday), matching `pd.PeriodIndex.weekday`.
    pub fn weekday(&self) -> Result<Vec<Option<u32>>, IndexError> {
        Ok(self.start_time()?.weekday())
    }

    /// Day of week (0=Monday, 6=Sunday), alias for weekday.
    /// Matches `pd.PeriodIndex.dayofweek`.
    pub fn dayofweek(&self) -> Result<Vec<Option<u32>>, IndexError> {
        self.weekday()
    }

    /// Day of week (0=Monday, 6=Sunday), alias for weekday.
    /// Matches `pd.PeriodIndex.day_of_week`.
    pub fn day_of_week(&self) -> Result<Vec<Option<u32>>, IndexError> {
        self.weekday()
    }

    /// Day of year (1-366), matching `pd.PeriodIndex.dayofyear`.
    pub fn dayofyear(&self) -> Result<Vec<Option<u32>>, IndexError> {
        Ok(self.start_time()?.dayofyear())
    }

    /// Day of year (1-366), alias for dayofyear.
    /// Matches `pd.PeriodIndex.day_of_year`.
    pub fn day_of_year(&self) -> Result<Vec<Option<u32>>, IndexError> {
        self.dayofyear()
    }

    /// Days in month (28-31), matching `pd.PeriodIndex.days_in_month`.
    pub fn days_in_month(&self) -> Result<Vec<Option<u32>>, IndexError> {
        Ok(self.start_time()?.daysinmonth())
    }

    /// Days in month (28-31), alias for days_in_month.
    /// Matches `pd.PeriodIndex.daysinmonth`.
    pub fn daysinmonth(&self) -> Result<Vec<Option<u32>>, IndexError> {
        self.days_in_month()
    }

    /// ISO week number (1-53), matching `pd.PeriodIndex.week`.
    pub fn week(&self) -> Result<Vec<Option<u32>>, IndexError> {
        Ok(self.start_time()?.week())
    }

    /// ISO week number (1-53), alias for week.
    /// Matches `pd.PeriodIndex.weekofyear`.
    pub fn weekofyear(&self) -> Result<Vec<Option<u32>>, IndexError> {
        self.week()
    }

    /// Whether year is a leap year, matching `pd.PeriodIndex.is_leap_year`.
    pub fn is_leap_year(&self) -> Result<Vec<Option<bool>>, IndexError> {
        Ok(self.start_time()?.is_leap_year())
    }

    /// Frequency resolution string, matching `pd.PeriodIndex.resolution`.
    #[must_use]
    pub fn resolution(&self) -> Option<&'static str> {
        self.values.first().map(|p| p.freq.resolution())
    }

    /// Format each period as a string with strftime, matching `pd.PeriodIndex.strftime`.
    pub fn strftime(&self, fmt: &str) -> Result<Vec<Option<String>>, IndexError> {
        Ok(self.start_time()?.strftime(fmt))
    }

    fn ensure_homogeneous_freq(&self) -> Result<Option<PeriodFreq>, IndexError> {
        let mut iter = self.values.iter();
        let Some(first) = iter.next() else {
            return Ok(None);
        };
        for period in iter {
            if period.freq != first.freq {
                return Err(IndexError::InvalidArgument(format!(
                    "PeriodIndex has mixed frequencies: {:?} and {:?}",
                    first.freq, period.freq
                )));
            }
        }
        Ok(Some(first.freq))
    }

    fn ensure_compatible_freq(&self, other: &Self) -> Result<(), IndexError> {
        if let (Some(left), Some(right)) = (self.values.first(), other.values.first())
            && left.freq != right.freq
        {
            return Err(IndexError::InvalidArgument(format!(
                "set operation: incompatible frequencies {:?} vs {:?}",
                left.freq, right.freq
            )));
        }
        self.ensure_homogeneous_freq()?;
        other.ensure_homogeneous_freq()?;
        Ok(())
    }

    /// Periods present in both, matching
    /// `pd.PeriodIndex.intersection(other)`. Mixed-freq rejects.
    pub fn intersection(&self, other: &Self) -> Result<Self, IndexError> {
        self.ensure_compatible_freq(other)?;
        let other_set: FxHashSet<&Period> = other.values.iter().collect();
        let mut seen = FxHashSet::<&Period>::default();
        let values: Vec<Period> = self
            .values
            .iter()
            .filter(|p| other_set.contains(p) && seen.insert(p))
            .copied()
            .collect();
        Ok(Self {
            values,
            name: if self.name == other.name {
                self.name.clone()
            } else {
                None
            },
        })
    }

    /// Self periods then other periods not seen, matching
    /// `pd.PeriodIndex.union(other)`. Mixed-freq rejects.
    pub fn union(&self, other: &Self) -> Result<Self, IndexError> {
        self.ensure_compatible_freq(other)?;
        let mut seen = FxHashSet::<Period>::default();
        let values: Vec<Period> = self
            .values
            .iter()
            .chain(other.values.iter())
            .filter(|p| seen.insert(**p))
            .copied()
            .collect();
        Ok(Self {
            values,
            name: if self.name == other.name {
                self.name.clone()
            } else {
                None
            },
        })
    }

    /// Self periods not in other, matching
    /// `pd.PeriodIndex.difference(other)`. Mixed-freq rejects.
    pub fn difference(&self, other: &Self) -> Result<Self, IndexError> {
        self.ensure_compatible_freq(other)?;
        let other_set: FxHashSet<&Period> = other.values.iter().collect();
        let mut seen = FxHashSet::<&Period>::default();
        let values: Vec<Period> = self
            .values
            .iter()
            .filter(|p| !other_set.contains(p) && seen.insert(p))
            .copied()
            .collect();
        Ok(Self {
            values,
            name: if self.name == other.name {
                self.name.clone()
            } else {
                None
            },
        })
    }

    /// Periods in either but not both, matching
    /// `pd.PeriodIndex.symmetric_difference(other)`. Mixed-freq rejects.
    pub fn symmetric_difference(&self, other: &Self) -> Result<Self, IndexError> {
        self.ensure_compatible_freq(other)?;
        let self_set: FxHashSet<&Period> = self.values.iter().collect();
        let other_set: FxHashSet<&Period> = other.values.iter().collect();
        let mut seen = FxHashSet::<Period>::default();
        let mut values = Vec::<Period>::new();
        for p in &self.values {
            if !other_set.contains(p) && seen.insert(*p) {
                values.push(*p);
            }
        }
        for p in &other.values {
            if !self_set.contains(p) && seen.insert(*p) {
                values.push(*p);
            }
        }
        Ok(Self {
            values,
            name: if self.name == other.name {
                self.name.clone()
            } else {
                None
            },
        })
    }

    /// Sort periods by ordinal ascending, matching
    /// `pd.PeriodIndex.sort_values()`. Mixed-frequency rejects.
    pub fn sort_values(&self) -> Result<Self, IndexError> {
        self.ensure_homogeneous_freq()?;
        let mut periods = self.values.clone();
        periods.sort_by_key(|period| period.ordinal);
        Ok(Self {
            values: periods,
            name: self.name.clone(),
        })
    }

    /// Alias for `sort_values`, matching `pd.PeriodIndex.sort()`.
    pub fn sort(&self) -> Result<Self, IndexError> {
        self.sort_values()
    }

    /// Position of the maximum ordinal, matching
    /// `pd.PeriodIndex.argmax()`. Mixed-freq input rejects; empty
    /// raises pandas-style "attempt to get argmax of an empty
    /// sequence".
    pub fn argmax(&self) -> Result<usize, IndexError> {
        self.ensure_homogeneous_freq()?;
        if self.values.is_empty() {
            return Err(IndexError::InvalidArgument(
                "attempt to get argmax of an empty sequence".to_owned(),
            ));
        }
        let mut best = 0;
        for (i, period) in self.values.iter().enumerate().skip(1) {
            if period.ordinal > self.values[best].ordinal {
                best = i;
            }
        }
        Ok(best)
    }

    /// Position of the minimum ordinal, matching
    /// `pd.PeriodIndex.argmin()`. Mixed-freq input rejects; empty
    /// raises pandas-style "attempt to get argmin of an empty
    /// sequence".
    pub fn argmin(&self) -> Result<usize, IndexError> {
        self.ensure_homogeneous_freq()?;
        if self.values.is_empty() {
            return Err(IndexError::InvalidArgument(
                "attempt to get argmin of an empty sequence".to_owned(),
            ));
        }
        let mut best = 0;
        for (i, period) in self.values.iter().enumerate().skip(1) {
            if period.ordinal < self.values[best].ordinal {
                best = i;
            }
        }
        Ok(best)
    }

    /// Positions that would sort the index by ordinal ascending,
    /// matching `pd.PeriodIndex.argsort()`. Mixed-freq input rejects.
    pub fn argsort(&self) -> Result<Vec<usize>, IndexError> {
        self.ensure_homogeneous_freq()?;
        let mut positions: Vec<usize> = (0..self.values.len()).collect();
        positions.sort_by_key(|&i| self.values[i].ordinal);
        Ok(positions)
    }

    /// Period with the mean ordinal, matching `pd.PeriodIndex.mean()`.
    /// Mixed-frequency input rejects.
    pub fn mean(&self) -> Result<Option<Period>, IndexError> {
        let freq = match self.ensure_homogeneous_freq()? {
            Some(f) => f,
            None => return Ok(None),
        };
        let total: i128 = self.values.iter().map(|p| i128::from(p.ordinal)).sum();
        let count = self.values.len() as i128;
        let avg = i64::try_from(total / count)
            .map_err(|_| IndexError::InvalidArgument("mean: ordinal overflow".to_owned()))?;
        Ok(Some(Period::new(avg, freq)))
    }

    /// Period with the median ordinal, matching `pd.PeriodIndex.median()`.
    /// For an even-length subset, returns the period at floor(median) of
    /// the two middle ordinals.
    pub fn median(&self) -> Result<Option<Period>, IndexError> {
        let freq = match self.ensure_homogeneous_freq()? {
            Some(f) => f,
            None => return Ok(None),
        };
        let mut ordinals: Vec<i64> = self.values.iter().map(|p| p.ordinal).collect();
        ordinals.sort_unstable();
        let mid = ordinals.len() / 2;
        let median = if ordinals.len() % 2 == 1 {
            ordinals[mid]
        } else {
            let total = i128::from(ordinals[mid - 1]) + i128::from(ordinals[mid]);
            i64::try_from(total / 2)
                .map_err(|_| IndexError::InvalidArgument("median: ordinal overflow".to_owned()))?
        };
        Ok(Some(Period::new(median, freq)))
    }

    /// Period with the smallest ordinal, matching `pd.PeriodIndex.min()`.
    /// Mixed-frequency input rejects because pandas requires same-freq
    /// comparisons; empty returns `Ok(None)` to mirror the pandas NaT result.
    pub fn min(&self) -> Result<Option<Period>, IndexError> {
        self.ensure_homogeneous_freq()?;
        Ok(self
            .values
            .iter()
            .copied()
            .min_by_key(|period| period.ordinal))
    }

    /// Period with the largest ordinal, matching `pd.PeriodIndex.max()`.
    pub fn max(&self) -> Result<Option<Period>, IndexError> {
        self.ensure_homogeneous_freq()?;
        Ok(self
            .values
            .iter()
            .copied()
            .max_by_key(|period| period.ordinal))
    }

    /// Binary-search insertion position, matching
    /// `pd.PeriodIndex.searchsorted(value, side)`. Mixed-frequency lookups
    /// reject because pandas requires same-freq comparisons. side must be
    /// `"left"` or `"right"`.
    pub fn searchsorted(&self, value: Period, side: &str) -> Result<usize, IndexError> {
        if side != "left" && side != "right" {
            return Err(IndexError::InvalidArgument(format!(
                "searchsorted: side must be 'left' or 'right', got {side:?}"
            )));
        }
        if let Some(first) = self.values.first()
            && first.freq != value.freq
        {
            return Err(IndexError::InvalidArgument(format!(
                "searchsorted: needle frequency {:?} does not match index frequency {:?}",
                value.freq, first.freq
            )));
        }
        let mut lo = 0usize;
        let mut hi = self.values.len();
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let cmp = self.values[mid].ordinal.cmp(&value.ordinal);
            use std::cmp::Ordering;
            let go_right = matches!(
                (cmp, side),
                (Ordering::Less, _) | (Ordering::Equal, "right")
            );
            if go_right {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        Ok(lo)
    }

    /// Half-open positional range for a period slice, matching
    /// `pd.PeriodIndex.slice_indexer(start, end)`.
    pub fn slice_indexer(
        &self,
        start: Period,
        end: Period,
    ) -> Result<std::ops::Range<usize>, IndexError> {
        let (left, right) = self.slice_locs(start, end)?;
        Ok(left..right)
    }

    /// Find positions of `[start, end]` for a period slice, matching
    /// `pd.PeriodIndex.slice_locs(start, end)`. Requires the index to
    /// be sorted ascending and the start/end periods to share its
    /// frequency.
    pub fn slice_locs(&self, start: Period, end: Period) -> Result<(usize, usize), IndexError> {
        if !self.is_monotonic_increasing() {
            return Err(IndexError::InvalidArgument(
                "slice_locs requires a monotonic increasing PeriodIndex".to_owned(),
            ));
        }
        let left = self.searchsorted(start, "left")?;
        let right = self.searchsorted(end, "right")?;
        Ok((left, right))
    }

    /// First position of `period`, matching
    /// `pd.PeriodIndex.get_loc(period)`.
    pub fn get_loc(&self, period: Period) -> Result<usize, IndexError> {
        self.values
            .iter()
            .position(|p| *p == period)
            .ok_or_else(|| {
                IndexError::InvalidArgument(format!("get_loc: period {period} not in PeriodIndex"))
            })
    }

    /// Set the index name, matching `pd.PeriodIndex.rename(name)`.
    #[must_use]
    pub fn rename(&self, name: &str) -> Self {
        self.set_name(name)
    }

    /// Reindex against `target`, matching
    /// `pd.PeriodIndex.reindex(target)`.
    #[must_use]
    pub fn reindex(&self, target: &Self) -> (Self, Vec<isize>) {
        let indexer = self.get_indexer(target.values());
        (target.clone(), indexer)
    }

    /// Locate every position matching each target, matching
    /// `pd.PeriodIndex.get_indexer_non_unique(targets)`.
    #[must_use]
    pub fn get_indexer_non_unique(&self, targets: &[Period]) -> (Vec<isize>, Vec<usize>) {
        let mut by_value = FxHashMap::<Period, Vec<usize>>::default();
        for (i, period) in self.values.iter().enumerate() {
            by_value.entry(*period).or_default().push(i);
        }
        let mut positions = Vec::<isize>::new();
        let mut missing = Vec::<usize>::new();
        for (idx, target) in targets.iter().enumerate() {
            if let Some(matches) = by_value.get(target) {
                positions.extend(
                    matches
                        .iter()
                        .map(|p| isize::try_from(*p).unwrap_or(isize::MAX)),
                );
            } else {
                positions.push(-1);
                missing.push(idx);
            }
        }
        (positions, missing)
    }

    /// Alias for [`get_indexer`], matching
    /// `pd.PeriodIndex.get_indexer_for(targets)`.
    #[must_use]
    pub fn get_indexer_for(&self, targets: &[Period]) -> Vec<isize> {
        self.get_indexer(targets)
    }

    /// Locate each target period in the index, matching
    /// `pd.PeriodIndex.get_indexer(targets)`. Returns `Vec<isize>`
    /// where `-1` means "missing".
    #[must_use]
    pub fn get_indexer(&self, targets: &[Period]) -> Vec<isize> {
        let mut positions = FxHashMap::<Period, isize>::default();
        for (i, period) in self.values.iter().enumerate() {
            positions
                .entry(*period)
                .or_insert_with(|| isize::try_from(i).unwrap_or(isize::MAX));
        }
        targets
            .iter()
            .map(|p| positions.get(p).copied().unwrap_or(-1))
            .collect()
    }

    /// Replace positions where `cond` is `false` with `other`, matching
    /// `pd.PeriodIndex.where(cond, other)`. The replacement period must
    /// share the index frequency.
    pub fn r#where(&self, cond: &[bool], other: Period) -> Result<Self, IndexError> {
        if cond.len() != self.values.len() {
            return Err(IndexError::LengthMismatch {
                expected: self.values.len(),
                actual: cond.len(),
                context: "where: cond length must match index length".to_owned(),
            });
        }
        if let Some(first) = self.values.first()
            && first.freq != other.freq
        {
            return Err(IndexError::InvalidArgument(format!(
                "where: replacement frequency {:?} does not match index frequency {:?}",
                other.freq, first.freq
            )));
        }
        let values: Vec<Period> = self
            .values
            .iter()
            .zip(cond.iter())
            .map(|(period, &keep)| if keep { *period } else { other })
            .collect();
        Ok(Self {
            values,
            name: self.name.clone(),
        })
    }

    /// Replace positions where `mask` is `true` with `value`, matching
    /// `pd.PeriodIndex.putmask(mask, value)`.
    pub fn putmask(&self, mask: &[bool], value: Period) -> Result<Self, IndexError> {
        if mask.len() != self.values.len() {
            return Err(IndexError::LengthMismatch {
                expected: self.values.len(),
                actual: mask.len(),
                context: "putmask: mask length must match index length".to_owned(),
            });
        }
        if let Some(first) = self.values.first()
            && first.freq != value.freq
        {
            return Err(IndexError::InvalidArgument(format!(
                "putmask: replacement frequency {:?} does not match index frequency {:?}",
                value.freq, first.freq
            )));
        }
        let values: Vec<Period> = self
            .values
            .iter()
            .zip(mask.iter())
            .map(|(period, &replace)| if replace { value } else { *period })
            .collect();
        Ok(Self {
            values,
            name: self.name.clone(),
        })
    }

    /// Insert `period` at position `loc`, matching
    /// `pd.PeriodIndex.insert(loc, period)`.
    pub fn insert(&self, loc: usize, period: Period) -> Result<Self, IndexError> {
        if loc > self.values.len() {
            return Err(IndexError::OutOfBounds {
                position: loc,
                length: self.values.len(),
            });
        }
        let mut periods = self.values.clone();
        periods.insert(loc, period);
        Ok(Self {
            values: periods,
            name: self.name.clone(),
        })
    }

    /// Shift each period by `n` units of its own frequency, matching
    /// `pd.PeriodIndex.shift(periods)`. Mixed-frequency input rejects
    /// via the existing ensure_homogeneous_freq guard.
    pub fn shift(&self, n: i64) -> Result<Self, IndexError> {
        self.ensure_homogeneous_freq()?;
        let values: Vec<Period> = self.values.iter().map(|p| p.shift(n)).collect();
        Ok(Self {
            values,
            name: self.name.clone(),
        })
    }

    /// Period labels are already discrete; pandas PeriodIndex.round returns a clone.
    #[must_use]
    pub fn round(&self, _freq: &str) -> Self {
        self.clone()
    }

    /// Whether period ordinals form a contiguous run, matching
    /// `pd.PeriodIndex.is_full`. Empty and single-element indexes are
    /// trivially full. Mixed-frequency input returns `false` because the
    /// concept is undefined.
    #[must_use]
    pub fn is_full(&self) -> bool {
        if self.values.len() <= 1 {
            return true;
        }
        // Mixed-freq: not full.
        let first_freq = self.values[0].freq;
        if self.values.iter().any(|p| p.freq != first_freq) {
            return false;
        }
        let mut sorted: Vec<i64> = self.values.iter().map(|p| p.ordinal).collect();
        sorted.sort_unstable();
        sorted.windows(2).all(|w| w[1] - w[0] == 1)
    }

    /// Stringify each period via Display, matching `pd.PeriodIndex.format()`.
    #[must_use]
    pub fn format(&self) -> Vec<String> {
        self.values.iter().map(Period::to_string).collect()
    }

    /// Frequency alias, matching `pd.PeriodIndex.freqstr`. Returns `None`
    /// for an empty index; otherwise the Display form of the freq.
    #[must_use]
    pub fn freqstr(&self) -> Option<String> {
        self.freq().map(|f| f.to_string())
    }

    /// Inferred frequency, matching `pd.PeriodIndex.inferred_freq`. Pandas
    /// returns the freq when all periods share it, otherwise `None`. Mixed
    /// frequency is detected via `ensure_homogeneous_freq` (the same guard
    /// used by min/max).
    #[must_use]
    pub fn inferred_freq(&self) -> Option<String> {
        match self.ensure_homogeneous_freq() {
            Ok(Some(freq)) => Some(freq.to_string()),
            Ok(None) | Err(_) => None,
        }
    }

    /// Convert to a flat [`Index`] of period strings, matching
    /// `pd.PeriodIndex.to_flat_index()`.
    #[must_use]
    pub fn to_flat_index(&self) -> Index {
        self.to_index()
    }

    /// String accessor for rendered period labels.
    #[must_use]
    pub fn r#str(&self) -> IndexStringAccessor<'_> {
        IndexStringAccessor::owned(self.to_flat_index())
    }

    /// One-column row materialization, matching `pd.PeriodIndex.to_frame(index=False)`.
    #[must_use]
    pub fn to_frame(&self) -> Vec<Vec<IndexLabel>> {
        self.to_flat_index().to_frame()
    }

    /// Series-shaped materialization using period labels as both index and values.
    #[must_use]
    pub fn to_series(&self) -> Vec<(IndexLabel, IndexLabel)> {
        self.to_flat_index().to_series()
    }

    /// Whether any period label coerces to true.
    #[must_use]
    pub fn any(&self) -> bool {
        self.to_flat_index().any()
    }

    /// Whether all period labels coerce to true.
    #[must_use]
    pub fn all(&self) -> bool {
        self.to_flat_index().all()
    }

    /// Get labels for a level. PeriodIndex is flat and only accepts level 0.
    pub fn get_level_values(&self, level: usize) -> Result<Index, IndexError> {
        self.to_flat_index().get_level_values(level)
    }

    /// Drop a level. PeriodIndex is flat, so removing its only level is invalid.
    pub fn droplevel(&self, level: usize) -> Result<Index, IndexError> {
        self.to_flat_index().droplevel(level)
    }

    /// Group equal period labels into position buckets.
    #[must_use]
    pub fn groupby(&self) -> HashMap<IndexLabel, Vec<usize>> {
        self.to_flat_index().groupby()
    }

    /// Apply a function to each period label, returning a flat Index.
    #[must_use]
    pub fn map<F>(&self, func: F) -> Index
    where
        F: Fn(&IndexLabel) -> IndexLabel,
    {
        self.to_flat_index().map(func)
    }

    /// Cast period labels to a pandas dtype string, returning a flat Index.
    pub fn astype(&self, dtype: &str) -> Result<Index, IndexError> {
        match dtype {
            "int" | "int64" => Ok(Index::from_i64(
                self.values.iter().map(|period| period.ordinal).collect(),
            )
            .set_names(self.name())),
            "datetime64[ns]" => Ok(Index::from_datetime64(
                self.values
                    .iter()
                    .copied()
                    .map(period_start_nanos)
                    .collect::<Result<Vec<_>, _>>()?,
            )
            .set_names(self.name())),
            _ => self.to_flat_index().astype(dtype),
        }
    }

    /// Nearest preceding-or-equal period label lookup.
    #[must_use]
    pub fn asof(&self, key: &IndexLabel) -> Option<IndexLabel> {
        self.to_flat_index().asof(key)
    }

    /// Locate nearest preceding-or-equal period positions for each target label.
    #[must_use]
    pub fn asof_locs(&self, where_index: &Index, mask: Option<&[bool]>) -> Vec<Option<usize>> {
        self.to_flat_index().asof_locs(where_index, mask)
    }

    /// Drop period labels, returning a flat Index.
    #[must_use]
    pub fn drop(&self, labels_to_drop: &[IndexLabel]) -> Index {
        self.to_flat_index().drop(labels_to_drop)
    }

    /// Join period labels with another flat Index.
    pub fn join(&self, other: &Index, how: &str) -> Result<Index, IndexError> {
        self.to_flat_index().join(other, how)
    }

    /// Sort period labels and return the positional sorter.
    #[must_use]
    pub fn sortlevel(&self) -> (Index, Vec<usize>) {
        self.to_flat_index().sortlevel()
    }

    /// Returns a clone, matching `pd.PeriodIndex.view()`.
    #[must_use]
    pub fn view(&self) -> Self {
        self.clone()
    }

    /// Identity transpose for a 1D index, matching
    /// `pd.PeriodIndex.transpose()`.
    #[must_use]
    pub fn transpose(&self) -> Self {
        self.clone()
    }

    /// Alias for `transpose`, matching `pd.PeriodIndex.T`.
    #[allow(non_snake_case)]
    #[must_use]
    pub fn T(&self) -> Self {
        self.transpose()
    }

    /// Flatten periods to a Vec<Period>, matching
    /// `pd.PeriodIndex.ravel()`.
    #[must_use]
    pub fn ravel(&self) -> Vec<Period> {
        self.values.clone()
    }

    /// Number of levels, matching `pd.PeriodIndex.nlevels`. Always `1`.
    #[must_use]
    pub fn nlevels(&self) -> usize {
        1
    }

    /// Identity dtype-reinference for typed indexes, matching
    /// `pd.PeriodIndex.infer_objects()`.
    #[must_use]
    pub fn infer_objects(&self) -> Self {
        self.clone()
    }

    /// Per-position membership mask, matching `pd.PeriodIndex.isin(values)`.
    #[must_use]
    pub fn isin(&self, values: &[Period]) -> Vec<bool> {
        let needle: FxHashSet<Period> = values.iter().copied().collect();
        self.values.iter().map(|p| needle.contains(p)).collect()
    }

    /// Concatenate with another PeriodIndex, matching
    /// `pd.PeriodIndex.append(other)`. Preserves the index name when both
    /// operands share it; otherwise the name is dropped.
    #[must_use]
    pub fn append(&self, other: &Self) -> Self {
        let mut periods = self.values.clone();
        periods.extend_from_slice(&other.values);
        let name = if self.name == other.name {
            self.name.clone()
        } else {
            None
        };
        Self {
            values: periods,
            name,
        }
    }

    /// Remove the period at the given position, matching
    /// `pd.PeriodIndex.delete(loc)`.
    pub fn delete(&self, loc: usize) -> Result<Self, IndexError> {
        if loc >= self.values.len() {
            return Err(IndexError::OutOfBounds {
                position: loc,
                length: self.values.len(),
            });
        }
        let mut periods = self.values.clone();
        periods.remove(loc);
        Ok(Self {
            values: periods,
            name: self.name.clone(),
        })
    }

    /// Factorize, matching `pd.PeriodIndex.factorize()`. Returns
    /// `(codes, uniques)` with isize codes — Period currently has no
    /// missing-value sentinel so all codes are non-negative.
    #[must_use]
    pub fn factorize(&self) -> (Vec<isize>, Self) {
        let mut positions = FxHashMap::<&Period, isize>::default();
        let mut uniques = Vec::<Period>::new();
        let mut codes = Vec::with_capacity(self.values.len());
        for period in &self.values {
            if let Some(code) = positions.get(period) {
                codes.push(*code);
            } else {
                let code = isize::try_from(uniques.len()).unwrap_or(isize::MAX);
                positions.insert(period, code);
                uniques.push(*period);
                codes.push(code);
            }
        }
        (
            codes,
            Self {
                values: uniques,
                name: self.name.clone(),
            },
        )
    }
}

/// Public pandas-style range index wrapper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RangeIndex {
    start: i64,
    stop: i64,
    step: i64,
    name: Option<String>,
}

impl RangeIndex {
    pub fn new(start: i64, stop: i64, step: i64) -> Result<Self, IndexError> {
        if step == 0 {
            return Err(IndexError::InvalidArgument(
                "RangeIndex step must be non-zero".to_owned(),
            ));
        }
        Ok(Self {
            start,
            stop,
            step,
            name: None,
        })
    }

    pub fn from_len(len: usize) -> Result<Self, IndexError> {
        let stop = i64::try_from(len).map_err(|_| {
            IndexError::InvalidArgument("RangeIndex length exceeds i64 range".to_owned())
        })?;
        Self::new(0, stop, 1)
    }

    #[must_use]
    pub const fn start(&self) -> i64 {
        self.start
    }

    #[must_use]
    pub const fn stop(&self) -> i64 {
        self.stop
    }

    #[must_use]
    pub const fn step(&self) -> i64 {
        self.step
    }

    #[must_use]
    pub fn len(&self) -> usize {
        let start = i128::from(self.start);
        let stop = i128::from(self.stop);
        let step = i128::from(self.step);
        let len = if step > 0 {
            if start >= stop {
                0
            } else {
                (stop - start + step - 1) / step
            }
        } else if start <= stop {
            0
        } else {
            let positive_step = -step;
            (start - stop + positive_step - 1) / positive_step
        };
        usize::try_from(len).unwrap_or(usize::MAX)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    #[must_use]
    pub fn set_name(&self, name: &str) -> Self {
        let mut out = self.clone();
        out.name = Some(name.to_owned());
        out
    }

    #[must_use]
    pub fn set_names(&self, name: Option<&str>) -> Self {
        let mut out = self.clone();
        out.name = name.map(str::to_owned);
        out
    }

    #[must_use]
    pub fn rename_index(&self, name: Option<&str>) -> Self {
        self.set_names(name)
    }

    #[must_use]
    pub fn names(&self) -> Vec<Option<String>> {
        vec![self.name.clone()]
    }

    #[must_use]
    pub fn copy(&self) -> Self {
        self.clone()
    }

    #[must_use]
    pub fn shape(&self) -> (usize,) {
        (self.len(),)
    }

    #[must_use]
    pub fn size(&self) -> usize {
        self.len()
    }

    #[must_use]
    pub fn empty(&self) -> bool {
        self.is_empty()
    }

    #[must_use]
    pub fn dtype(&self) -> &'static str {
        "int64"
    }

    #[must_use]
    pub fn dtypes(&self) -> Vec<&'static str> {
        vec![self.dtype()]
    }

    #[must_use]
    pub fn memory_usage(&self, _deep: bool) -> usize {
        self.len() * std::mem::size_of::<i64>()
    }

    #[must_use]
    pub fn nbytes(&self) -> usize {
        self.memory_usage(false)
    }

    #[must_use]
    pub fn is_unique(&self) -> bool {
        true
    }

    #[must_use]
    pub fn has_duplicates(&self) -> bool {
        false
    }

    #[must_use]
    pub fn is_monotonic_increasing(&self) -> bool {
        self.len() <= 1 || self.step > 0
    }

    #[must_use]
    pub fn is_monotonic(&self) -> bool {
        self.is_monotonic_increasing()
    }

    #[must_use]
    pub fn is_monotonic_decreasing(&self) -> bool {
        self.len() <= 1 || self.step < 0
    }

    #[must_use]
    pub fn nunique(&self) -> usize {
        self.len()
    }

    #[must_use]
    pub fn ndim(&self) -> usize {
        1
    }

    pub fn item(&self) -> Result<i64, IndexError> {
        if self.len() == 1 {
            Ok(self.start)
        } else {
            Err(IndexError::InvalidArgument(format!(
                "item requires exactly one label, got {}",
                self.len()
            )))
        }
    }

    #[must_use]
    pub fn is_(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }

    #[must_use]
    pub fn equals(&self, other: &Self) -> bool {
        self.values() == other.values()
    }

    #[must_use]
    pub fn identical(&self, other: &Self) -> bool {
        self.equals(other) && self.name == other.name
    }

    #[must_use]
    pub fn holds_integer(&self) -> bool {
        true
    }

    #[must_use]
    pub fn inferred_type(&self) -> &'static str {
        if self.is_empty() { "empty" } else { "integer" }
    }

    #[must_use]
    pub fn is_boolean(&self) -> bool {
        false
    }

    #[must_use]
    pub fn is_categorical(&self) -> bool {
        false
    }

    #[must_use]
    pub fn is_floating(&self) -> bool {
        false
    }

    #[must_use]
    pub fn is_integer(&self) -> bool {
        true
    }

    #[must_use]
    pub fn is_interval(&self) -> bool {
        false
    }

    #[must_use]
    pub fn is_numeric(&self) -> bool {
        true
    }

    #[must_use]
    pub fn is_object(&self) -> bool {
        false
    }

    #[must_use]
    pub fn to_index(&self) -> Index {
        Index::from_range(self.start, self.stop, self.step).set_names(self.name.as_deref())
    }

    #[must_use]
    pub fn values(&self) -> Vec<i64> {
        self.to_index()
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Int64(value) => Some(*value),
                IndexLabel::Float64(_)
                | IndexLabel::Bool(_)
                | IndexLabel::Utf8(_)
                | IndexLabel::Timedelta64(_)
                | IndexLabel::Datetime64(_)
                | IndexLabel::Null(_) => None,
            })
            .collect()
    }

    /// Positional first differences for RangeIndex values.
    #[must_use]
    pub fn diff(&self, periods: i64) -> Vec<Option<i64>> {
        let values = self.values();
        positional_diff(values.len(), periods, |current, previous| {
            values[current].checked_sub(values[previous])
        })
    }

    #[must_use]
    pub fn to_list(&self) -> Vec<i64> {
        self.values()
    }

    #[must_use]
    pub fn tolist(&self) -> Vec<i64> {
        self.values()
    }

    #[must_use]
    pub fn to_numpy(&self) -> Vec<i64> {
        self.values()
    }

    #[must_use]
    pub fn array(&self) -> Vec<i64> {
        self.values()
    }

    /// Position of the maximum value, matching `pd.RangeIndex.argmax()`.
    ///
    /// `step > 0` makes the last position the maximum; `step < 0` makes
    /// position 0 the maximum. Empty ranges raise the same
    /// `attempt to get argmax of an empty sequence` error pandas surfaces.
    pub fn argmax(&self) -> Result<usize, IndexError> {
        if self.is_empty() {
            return Err(IndexError::InvalidArgument(
                "attempt to get argmax of an empty sequence".to_owned(),
            ));
        }
        if self.step > 0 {
            Ok(self.len() - 1)
        } else {
            Ok(0)
        }
    }

    /// Position of the minimum value, matching `pd.RangeIndex.argmin()`.
    pub fn argmin(&self) -> Result<usize, IndexError> {
        if self.is_empty() {
            return Err(IndexError::InvalidArgument(
                "attempt to get argmin of an empty sequence".to_owned(),
            ));
        }
        if self.step > 0 {
            Ok(0)
        } else {
            Ok(self.len() - 1)
        }
    }

    /// Positions that would sort the index ascending, matching
    /// `pd.RangeIndex.argsort()`.
    #[must_use]
    pub fn argsort(&self) -> Vec<usize> {
        let len = self.len();
        if self.step >= 0 {
            (0..len).collect()
        } else {
            (0..len).rev().collect()
        }
    }

    /// RangeIndex enforces uniqueness, so every position is reported as a
    /// non-duplicate, matching `pd.RangeIndex.duplicated(keep=...)`.
    #[must_use]
    pub fn duplicated(&self, _keep: DuplicateKeep) -> Vec<bool> {
        vec![false; self.len()]
    }

    /// Drop duplicates, matching `pd.RangeIndex.drop_duplicates()`.
    /// Returns a clone because RangeIndex never has duplicates.
    #[must_use]
    pub fn drop_duplicates(&self) -> Self {
        self.clone()
    }

    /// Per-position missingness mask, matching `pd.RangeIndex.isna()`.
    /// Always all-false because RangeIndex is int64-typed.
    #[must_use]
    pub fn isna(&self) -> Vec<bool> {
        vec![false; self.len()]
    }

    /// Alias for [`isna`], matching `pd.RangeIndex.isnull()`.
    #[must_use]
    pub fn isnull(&self) -> Vec<bool> {
        self.isna()
    }

    /// Per-position non-missing mask, matching `pd.RangeIndex.notna()`.
    #[must_use]
    pub fn notna(&self) -> Vec<bool> {
        vec![true; self.len()]
    }

    /// Alias for [`notna`], matching `pd.RangeIndex.notnull()`.
    #[must_use]
    pub fn notnull(&self) -> Vec<bool> {
        self.notna()
    }

    /// Whether any position is missing, matching `pd.RangeIndex.hasnans`.
    #[must_use]
    pub fn hasnans(&self) -> bool {
        false
    }

    /// Drop missing positions, matching `pd.RangeIndex.dropna()`.
    /// Returns a clone because RangeIndex cannot hold missing values.
    #[must_use]
    pub fn dropna(&self) -> Self {
        self.clone()
    }

    /// Fill missing positions, matching `pd.RangeIndex.fillna(value)`.
    /// Returns a clone — RangeIndex has no missing positions to fill.
    #[must_use]
    pub fn fillna(&self, _value: i64) -> Self {
        self.clone()
    }

    /// Stringify each value, matching `pd.RangeIndex.format()`.
    #[must_use]
    pub fn format(&self) -> Vec<String> {
        self.values().into_iter().map(|v| v.to_string()).collect()
    }

    /// Identity factorization, matching `pd.RangeIndex.factorize()`.
    /// Codes are [0..len) because every value is unique; uniques is a
    /// clone of `self`.
    #[must_use]
    pub fn factorize(&self) -> (Vec<usize>, Self) {
        ((0..self.len()).collect(), self.clone())
    }

    /// Pick values at the given positions, matching
    /// `pd.RangeIndex.take()`. Out-of-bounds positions raise
    /// [`IndexError::OutOfBounds`].
    pub fn take(&self, positions: &[usize]) -> Result<Index, IndexError> {
        let values = self.values();
        for &p in positions {
            if p >= values.len() {
                return Err(IndexError::OutOfBounds {
                    position: p,
                    length: values.len(),
                });
            }
        }
        let labels: Vec<IndexLabel> = positions
            .iter()
            .map(|&p| IndexLabel::Int64(values[p]))
            .collect();
        let mut idx = Index::new(labels);
        if let Some(name) = self.name() {
            idx = idx.set_name(name);
        }
        Ok(idx)
    }

    /// Repeat each value `repeats` times, matching `pd.RangeIndex.repeat()`.
    /// Returns a flat [`Index`] because the result is generally not a
    /// contiguous range.
    #[must_use]
    pub fn repeat(&self, repeats: usize) -> Index {
        let mut labels = Vec::with_capacity(self.len() * repeats);
        for value in self.values() {
            for _ in 0..repeats {
                labels.push(IndexLabel::Int64(value));
            }
        }
        let mut idx = Index::new(labels);
        if let Some(name) = self.name() {
            idx = idx.set_name(name);
        }
        idx
    }

    /// First and last value as (start, last), or None if empty. Used to
    /// power closed-form reductions that don't materialize the full vector.
    fn first_last(&self) -> Option<(i64, i64)> {
        let len = self.len();
        if len == 0 {
            return None;
        }
        let last = self.start + (len as i64 - 1) * self.step;
        Some((self.start, last))
    }

    /// Sort values ascending, matching `pd.RangeIndex.sort_values()`.
    /// Ascending or zero step returns a clone; descending step rebuilds
    /// an ascending RangeIndex starting from min with positive step.
    /// Empty returns clone of self.
    #[must_use]
    pub fn sort_values(&self) -> Self {
        if self.is_empty() || self.step >= 0 {
            return self.clone();
        }
        let len = self.len();
        let last = self.start + (len as i64 - 1) * self.step;
        let new_step = -self.step;
        let new_stop = last + (len as i64) * new_step;
        Self {
            start: last,
            stop: new_stop,
            step: new_step,
            name: self.name.clone(),
        }
    }

    /// Alias for `sort_values`, matching `pd.RangeIndex.sort()`.
    #[must_use]
    pub fn sort(&self) -> Self {
        self.sort_values()
    }

    /// Smallest value in the range, matching `pd.RangeIndex.min()`. Closed
    /// form on (start, step, len). Empty returns None.
    #[must_use]
    pub fn min(&self) -> Option<i64> {
        let (first, last) = self.first_last()?;
        Some(first.min(last))
    }

    /// Largest value in the range, matching `pd.RangeIndex.max()`.
    #[must_use]
    pub fn max(&self) -> Option<i64> {
        let (first, last) = self.first_last()?;
        Some(first.max(last))
    }

    /// Median value, matching `pd.RangeIndex.median()`. Returns `None`
    /// for an empty range; for an even-length range, returns the average
    /// of the two middle values as f64.
    #[must_use]
    pub fn median(&self) -> Option<f64> {
        let len = self.len();
        if len == 0 {
            return None;
        }
        let values = self.values();
        let mid = len / 2;
        if len % 2 == 1 {
            Some(values[mid] as f64)
        } else {
            Some((values[mid - 1] as f64 + values[mid] as f64) / 2.0)
        }
    }

    /// Sample variance (ddof=1), matching `pd.RangeIndex.var()`. Returns
    /// `None` for fewer than two values.
    #[must_use]
    pub fn var(&self) -> Option<f64> {
        let values: Vec<f64> = self.values().into_iter().map(|v| v as f64).collect();
        if values.len() < 2 {
            return None;
        }
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        Some(values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (values.len() as f64 - 1.0))
    }

    /// Sample standard deviation (ddof=1), matching `pd.RangeIndex.std()`.
    #[must_use]
    pub fn std(&self) -> Option<f64> {
        self.var().map(f64::sqrt)
    }

    /// Product of all values, matching `pd.RangeIndex.prod()`. Empty
    /// returns 1; saturating to i64 on overflow.
    #[must_use]
    pub fn prod(&self) -> i64 {
        let mut total: i128 = 1;
        for v in self.values() {
            total = total.saturating_mul(i128::from(v));
        }
        i64::try_from(total).unwrap_or(if total > 0 { i64::MAX } else { i64::MIN })
    }

    /// Sum of all values, matching `pd.RangeIndex.sum()`. Closed form via
    /// arithmetic-progression: `n * (first + last) / 2` when `n*(first+last)`
    /// is even; falls back to a precise i128 path otherwise.
    #[must_use]
    pub fn sum(&self) -> i64 {
        let len = self.len();
        if len == 0 {
            return 0;
        }
        let Some((first, last)) = self.first_last() else {
            return 0;
        };
        let n = i128::from(len as i64);
        let total = (i128::from(first) + i128::from(last)) * n / 2;
        i64::try_from(total).unwrap_or(i64::MAX)
    }

    /// Mean of all values, matching `pd.RangeIndex.mean()`. Returns `None`
    /// for an empty range.
    #[must_use]
    pub fn mean(&self) -> Option<f64> {
        let len = self.len();
        if len == 0 {
            return None;
        }
        let (first, last) = self.first_last()?;
        Some((first as f64 + last as f64) / 2.0)
    }

    /// Binary-search insertion position, matching
    /// `pd.RangeIndex.searchsorted(value, side)`. Restricted to
    /// ascending ranges (`step > 0`) because searchsorted assumes a
    /// monotonically-increasing input; negative-step ranges raise.
    pub fn searchsorted(&self, value: i64, side: &str) -> Result<usize, IndexError> {
        if side != "left" && side != "right" {
            return Err(IndexError::InvalidArgument(format!(
                "searchsorted: side must be 'left' or 'right', got {side:?}"
            )));
        }
        if self.step < 0 {
            return Err(IndexError::InvalidArgument(
                "searchsorted requires a monotonically-increasing RangeIndex".to_owned(),
            ));
        }
        let values = self.values();
        let mut lo = 0usize;
        let mut hi = values.len();
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let cmp = values[mid].cmp(&value);
            use std::cmp::Ordering;
            let go_right = matches!(
                (cmp, side),
                (Ordering::Less, _) | (Ordering::Equal, "right")
            );
            if go_right {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        Ok(lo)
    }

    /// Convert to a flat [`Index`] of i64 labels, matching
    /// `pd.RangeIndex.to_flat_index()`.
    #[must_use]
    pub fn to_flat_index(&self) -> Index {
        let labels: Vec<IndexLabel> = self.values().into_iter().map(IndexLabel::Int64).collect();
        let mut idx = Index::new(labels);
        if let Some(name) = self.name() {
            idx = idx.set_name(name);
        }
        idx
    }

    /// String accessor for the flat integer labels.
    #[must_use]
    pub fn r#str(&self) -> IndexStringAccessor<'_> {
        IndexStringAccessor::owned(self.to_flat_index())
    }

    /// One-column row materialization, matching `pd.RangeIndex.to_frame(index=False)`.
    #[must_use]
    pub fn to_frame(&self) -> Vec<Vec<IndexLabel>> {
        self.to_flat_index().to_frame()
    }

    /// Series-shaped materialization using range labels as both index and values.
    #[must_use]
    pub fn to_series(&self) -> Vec<(IndexLabel, IndexLabel)> {
        self.to_flat_index().to_series()
    }

    /// Whether any range label coerces to true.
    #[must_use]
    pub fn any(&self) -> bool {
        self.to_flat_index().any()
    }

    /// Whether all range labels coerce to true.
    #[must_use]
    pub fn all(&self) -> bool {
        self.to_flat_index().all()
    }

    /// Get labels for a level. RangeIndex is flat and only accepts level 0.
    pub fn get_level_values(&self, level: usize) -> Result<Index, IndexError> {
        self.to_flat_index().get_level_values(level)
    }

    /// Drop a level. RangeIndex is flat, so removing its only level is invalid.
    pub fn droplevel(&self, level: usize) -> Result<Index, IndexError> {
        self.to_flat_index().droplevel(level)
    }

    /// Group equal range labels into position buckets.
    #[must_use]
    pub fn groupby(&self) -> HashMap<IndexLabel, Vec<usize>> {
        self.to_flat_index().groupby()
    }

    /// Apply a function to each range label, returning a flat Index.
    #[must_use]
    pub fn map<F>(&self, func: F) -> Index
    where
        F: Fn(&IndexLabel) -> IndexLabel,
    {
        self.to_flat_index().map(func)
    }

    /// Cast range labels to a pandas dtype string, returning a flat Index.
    pub fn astype(&self, dtype: &str) -> Result<Index, IndexError> {
        self.to_flat_index().astype(dtype)
    }

    /// Nearest preceding-or-equal range label lookup.
    #[must_use]
    pub fn asof(&self, key: &IndexLabel) -> Option<IndexLabel> {
        self.to_flat_index().asof(key)
    }

    /// Locate nearest preceding-or-equal range positions for each target label.
    #[must_use]
    pub fn asof_locs(&self, where_index: &Index, mask: Option<&[bool]>) -> Vec<Option<usize>> {
        self.to_flat_index().asof_locs(where_index, mask)
    }

    /// Drop range labels, returning a flat Index.
    #[must_use]
    pub fn drop(&self, labels_to_drop: &[IndexLabel]) -> Index {
        self.to_flat_index().drop(labels_to_drop)
    }

    /// Join range labels with another flat Index.
    pub fn join(&self, other: &Index, how: &str) -> Result<Index, IndexError> {
        self.to_flat_index().join(other, how)
    }

    /// Sort range labels and return the positional sorter.
    #[must_use]
    pub fn sortlevel(&self) -> (Index, Vec<usize>) {
        self.to_flat_index().sortlevel()
    }

    /// Returns a clone, matching `pd.RangeIndex.view()`.
    #[must_use]
    pub fn view(&self) -> Self {
        self.clone()
    }

    /// Identity transpose for a 1D index, matching
    /// `pd.RangeIndex.transpose()`.
    #[must_use]
    pub fn transpose(&self) -> Self {
        self.clone()
    }

    /// Alias for `transpose`, matching `pd.RangeIndex.T`.
    #[allow(non_snake_case)]
    #[must_use]
    pub fn T(&self) -> Self {
        self.transpose()
    }

    /// Flatten the range to a Vec<i64>, matching `pd.RangeIndex.ravel()`.
    #[must_use]
    pub fn ravel(&self) -> Vec<i64> {
        self.values()
    }

    /// Number of levels, matching `pd.RangeIndex.nlevels`. Always `1`.
    #[must_use]
    pub fn nlevels(&self) -> usize {
        1
    }

    /// Identity dtype-reinference, matching `pd.RangeIndex.infer_objects()`.
    #[must_use]
    pub fn infer_objects(&self) -> Self {
        self.clone()
    }

    /// Per-position membership mask, matching `pd.RangeIndex.isin(values)`.
    #[must_use]
    pub fn isin(&self, values: &[i64]) -> Vec<bool> {
        let needle: FxHashSet<i64> = values.iter().copied().collect();
        self.values().iter().map(|v| needle.contains(v)).collect()
    }

    /// Half-open positional range for a value slice, matching
    /// `pd.RangeIndex.slice_indexer(start, end)`.
    pub fn slice_indexer(
        &self,
        start: i64,
        end: i64,
    ) -> Result<std::ops::Range<usize>, IndexError> {
        let (left, right) = self.slice_locs(start, end)?;
        Ok(left..right)
    }

    /// Find positions of `[start, end]` for a value slice, matching
    /// `pd.RangeIndex.slice_locs(start, end)`. Requires the range to
    /// be ascending (`step > 0`).
    pub fn slice_locs(&self, start: i64, end: i64) -> Result<(usize, usize), IndexError> {
        if self.step < 0 {
            return Err(IndexError::InvalidArgument(
                "slice_locs requires a monotonic increasing RangeIndex".to_owned(),
            ));
        }
        let left = self.searchsorted(start, "left")?;
        let right = self.searchsorted(end, "right")?;
        Ok((left, right))
    }

    /// First position of `value`, matching `pd.RangeIndex.get_loc(value)`.
    /// Closed-form on (start, step, len).
    pub fn get_loc(&self, value: i64) -> Result<usize, IndexError> {
        if self.step == 0 {
            return Err(IndexError::InvalidArgument(
                "get_loc: zero-step RangeIndex is invalid".to_owned(),
            ));
        }
        let offset = value - self.start;
        if offset.checked_rem_euclid(self.step) != Some(0) {
            return Err(IndexError::InvalidArgument(format!(
                "get_loc: {value} not in RangeIndex"
            )));
        }
        let pos = offset / self.step;
        if pos < 0 || (pos as usize) >= self.len() {
            return Err(IndexError::InvalidArgument(format!(
                "get_loc: {value} not in RangeIndex"
            )));
        }
        Ok(pos as usize)
    }

    /// Set the index name, matching `pd.RangeIndex.rename(name)`.
    #[must_use]
    pub fn rename(&self, name: &str) -> Self {
        self.set_name(name)
    }

    /// Reindex against `target`, matching `pd.RangeIndex.reindex(target)`.
    /// Returns `(target.clone(), indexer)`.
    #[must_use]
    pub fn reindex(&self, target: &Self) -> (Self, Vec<isize>) {
        let indexer = self.get_indexer(&target.values());
        (target.clone(), indexer)
    }

    /// Locate every position matching each target, matching
    /// `pd.RangeIndex.get_indexer_non_unique(targets)`. RangeIndex is
    /// always unique so each target either matches one position or
    /// none.
    #[must_use]
    pub fn get_indexer_non_unique(&self, targets: &[i64]) -> (Vec<isize>, Vec<usize>) {
        let mut positions = Vec::<isize>::new();
        let mut missing = Vec::<usize>::new();
        for (idx, target) in targets.iter().enumerate() {
            match self.get_loc(*target) {
                Ok(p) => positions.push(p as isize),
                Err(_) => {
                    positions.push(-1);
                    missing.push(idx);
                }
            }
        }
        (positions, missing)
    }

    /// Alias for [`get_indexer`], matching
    /// `pd.RangeIndex.get_indexer_for(targets)`.
    #[must_use]
    pub fn get_indexer_for(&self, targets: &[i64]) -> Vec<isize> {
        self.get_indexer(targets)
    }

    /// Locate each target value, matching
    /// `pd.RangeIndex.get_indexer(targets)`. Closed-form per target.
    #[must_use]
    pub fn get_indexer(&self, targets: &[i64]) -> Vec<isize> {
        targets
            .iter()
            .map(|&v| self.get_loc(v).map(|p| p as isize).unwrap_or(-1))
            .collect()
    }

    /// Replace positions where `cond` is `false` with `other`, matching
    /// `pd.RangeIndex.where(cond, other)`. Returns flat Index because
    /// the result is generally not a contiguous range.
    pub fn r#where(&self, cond: &[bool], other: i64) -> Result<Index, IndexError> {
        let values = self.values();
        if cond.len() != values.len() {
            return Err(IndexError::LengthMismatch {
                expected: values.len(),
                actual: cond.len(),
                context: "where: cond length must match index length".to_owned(),
            });
        }
        let labels: Vec<IndexLabel> = values
            .into_iter()
            .zip(cond.iter())
            .map(|(v, &keep)| IndexLabel::Int64(if keep { v } else { other }))
            .collect();
        let mut out = Index::new(labels);
        if let Some(name) = self.name() {
            out = out.set_name(name);
        }
        Ok(out)
    }

    /// Replace positions where `mask` is `true` with `value`, matching
    /// `pd.RangeIndex.putmask(mask, value)`.
    pub fn putmask(&self, mask: &[bool], value: i64) -> Result<Index, IndexError> {
        let values = self.values();
        if mask.len() != values.len() {
            return Err(IndexError::LengthMismatch {
                expected: values.len(),
                actual: mask.len(),
                context: "putmask: mask length must match index length".to_owned(),
            });
        }
        let labels: Vec<IndexLabel> = values
            .into_iter()
            .zip(mask.iter())
            .map(|(v, &replace)| IndexLabel::Int64(if replace { value } else { v }))
            .collect();
        let mut out = Index::new(labels);
        if let Some(name) = self.name() {
            out = out.set_name(name);
        }
        Ok(out)
    }

    fn set_op_via_int<F>(&self, other: &Self, op: F) -> Index
    where
        F: FnOnce(Vec<i64>, Vec<i64>) -> Vec<i64>,
    {
        let values = op(self.values(), other.values());
        let labels: Vec<IndexLabel> = values.into_iter().map(IndexLabel::Int64).collect();
        let mut idx = Index::new(labels);
        if let Some(name) = self.name().filter(|_| self.name() == other.name()) {
            idx = idx.set_name(name);
        }
        idx
    }

    /// Values present in both ranges, matching
    /// `pd.RangeIndex.intersection(other)`. Returns flat Index because
    /// the result may not be a contiguous range.
    #[must_use]
    pub fn intersection(&self, other: &Self) -> Index {
        self.set_op_via_int(other, |left, right| {
            let right_set: FxHashSet<i64> = right.into_iter().collect();
            let mut seen = FxHashSet::<i64>::default();
            left.into_iter()
                .filter(|v| right_set.contains(v) && seen.insert(*v))
                .collect()
        })
    }

    /// Self values then other values not seen, matching
    /// `pd.RangeIndex.union(other)`.
    #[must_use]
    pub fn union(&self, other: &Self) -> Index {
        self.set_op_via_int(other, |left, right| {
            let mut seen = FxHashSet::<i64>::default();
            left.into_iter()
                .chain(right)
                .filter(|v| seen.insert(*v))
                .collect()
        })
    }

    /// Self values not in other, matching
    /// `pd.RangeIndex.difference(other)`.
    #[must_use]
    pub fn difference(&self, other: &Self) -> Index {
        // Per br-frankenpandas-6r1lq: difference preserves self.name (not
        // shared_name like union/intersection). Build inline rather than
        // routing through set_op_via_int's shared-name logic.
        let right_set: FxHashSet<i64> = other.values().into_iter().collect();
        let mut seen = FxHashSet::<i64>::default();
        let labels: Vec<IndexLabel> = self
            .values()
            .into_iter()
            .filter(|v| !right_set.contains(v) && seen.insert(*v))
            .map(IndexLabel::Int64)
            .collect();
        let mut idx = Index::new(labels);
        if let Some(name) = self.name() {
            idx = idx.set_name(name);
        }
        idx
    }

    /// Values in either but not both, matching
    /// `pd.RangeIndex.symmetric_difference(other)`.
    #[must_use]
    pub fn symmetric_difference(&self, other: &Self) -> Index {
        self.set_op_via_int(other, |left, right| {
            let left_set: FxHashSet<i64> = left.iter().copied().collect();
            let right_set: FxHashSet<i64> = right.iter().copied().collect();
            let mut seen = FxHashSet::<i64>::default();
            let mut out = Vec::new();
            for v in left {
                if !right_set.contains(&v) && seen.insert(v) {
                    out.push(v);
                }
            }
            for v in right {
                if !left_set.contains(&v) && seen.insert(v) {
                    out.push(v);
                }
            }
            out
        })
    }

    /// Insert `value` at position `loc`, matching
    /// `pd.RangeIndex.insert(loc, value)`. Returns a flat [`Index`]
    /// because the result is generally not a contiguous range.
    pub fn insert(&self, loc: usize, value: i64) -> Result<Index, IndexError> {
        let values = self.values();
        if loc > values.len() {
            return Err(IndexError::OutOfBounds {
                position: loc,
                length: values.len(),
            });
        }
        let mut labels: Vec<IndexLabel> = values.into_iter().map(IndexLabel::Int64).collect();
        labels.insert(loc, IndexLabel::Int64(value));
        let mut out = Index::new(labels);
        if let Some(name) = self.name() {
            out = out.set_name(name);
        }
        Ok(out)
    }

    /// Concatenate with another RangeIndex, matching
    /// `pd.RangeIndex.append(other)`. Returns a flat [`Index`] because the
    /// resulting values are generally not a contiguous range; preserves
    /// the index name when both operands share it.
    #[must_use]
    pub fn append(&self, other: &Self) -> Index {
        let mut labels: Vec<IndexLabel> =
            self.values().into_iter().map(IndexLabel::Int64).collect();
        labels.extend(other.values().into_iter().map(IndexLabel::Int64));
        let mut out = Index::new(labels);
        if let Some(name) = self.name().filter(|_| self.name() == other.name()) {
            out = out.set_name(name);
        }
        out
    }

    /// Remove the value at the given position, matching
    /// `pd.RangeIndex.delete(loc)`. Returns a flat [`Index`] because the
    /// residual values may no longer form a contiguous range.
    pub fn delete(&self, loc: usize) -> Result<Index, IndexError> {
        let values = self.values();
        if loc >= values.len() {
            return Err(IndexError::OutOfBounds {
                position: loc,
                length: values.len(),
            });
        }
        let labels: Vec<IndexLabel> = values
            .into_iter()
            .enumerate()
            .filter(|(i, _)| *i != loc)
            .map(|(_, v)| IndexLabel::Int64(v))
            .collect();
        let mut out = Index::new(labels);
        if let Some(name) = self.name() {
            out = out.set_name(name);
        }
        Ok(out)
    }
}

/// Public pandas-style categorical index wrapper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoricalIndex {
    labels: Vec<String>,
    categories: Vec<String>,
    ordered: bool,
    name: Option<String>,
}

impl CategoricalIndex {
    #[must_use]
    pub fn from_values(labels: Vec<String>, ordered: bool) -> Self {
        // First-seen dedup in O(n): a side hash set tracks membership while the
        // categories Vec preserves insertion order, replacing the O(n·k)
        // `categories.contains` linear rescan per label.
        let mut categories = Vec::<String>::new();
        let mut seen: FxHashSet<&str> = FxHashSet::default();
        for label in &labels {
            if seen.insert(label.as_str()) {
                categories.push(label.clone());
            }
        }
        Self {
            labels,
            categories,
            ordered,
            name: None,
        }
    }

    pub fn with_categories(
        labels: Vec<String>,
        categories: Vec<String>,
        ordered: bool,
    ) -> Result<Self, IndexError> {
        // O(n+k) membership: hash the category set once, then validate each
        // label in original order (first offending label still reported).
        let category_set: FxHashSet<&str> = categories.iter().map(String::as_str).collect();
        for label in &labels {
            if !category_set.contains(label.as_str()) {
                return Err(IndexError::InvalidArgument(format!(
                    "CategoricalIndex label {label:?} is not present in categories"
                )));
            }
        }
        Ok(Self {
            labels,
            categories,
            ordered,
            name: None,
        })
    }

    #[must_use]
    pub fn labels(&self) -> &[String] {
        &self.labels
    }

    #[must_use]
    pub fn categories(&self) -> &[String] {
        &self.categories
    }

    #[must_use]
    pub fn ordered(&self) -> bool {
        self.ordered
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.labels.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.labels.is_empty()
    }

    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    #[must_use]
    pub fn set_name(&self, name: &str) -> Self {
        let mut out = self.clone();
        out.name = Some(name.to_owned());
        out
    }

    #[must_use]
    pub fn set_names(&self, name: Option<&str>) -> Self {
        let mut out = self.clone();
        out.name = name.map(str::to_owned);
        out
    }

    #[must_use]
    pub fn rename_index(&self, name: Option<&str>) -> Self {
        self.set_names(name)
    }

    #[must_use]
    pub fn names(&self) -> Vec<Option<String>> {
        vec![self.name.clone()]
    }

    #[must_use]
    pub fn copy(&self) -> Self {
        self.clone()
    }

    #[must_use]
    pub fn shape(&self) -> (usize,) {
        (self.len(),)
    }

    #[must_use]
    pub fn size(&self) -> usize {
        self.len()
    }

    #[must_use]
    pub fn empty(&self) -> bool {
        self.is_empty()
    }

    #[must_use]
    pub fn dtype(&self) -> &'static str {
        "category"
    }

    #[must_use]
    pub fn dtypes(&self) -> Vec<&'static str> {
        vec![self.dtype()]
    }

    #[must_use]
    pub fn memory_usage(&self, deep: bool) -> usize {
        let fixed = (self.labels.len() + self.categories.len()) * std::mem::size_of::<String>();
        if deep {
            fixed
                + self.labels.iter().map(String::len).sum::<usize>()
                + self.categories.iter().map(String::len).sum::<usize>()
                + self.name.as_ref().map_or(0, String::len)
        } else {
            fixed
        }
    }

    #[must_use]
    pub fn nbytes(&self) -> usize {
        self.memory_usage(false)
    }

    #[must_use]
    pub fn isna(&self) -> Vec<bool> {
        vec![false; self.len()]
    }

    #[must_use]
    pub fn notna(&self) -> Vec<bool> {
        vec![true; self.len()]
    }

    /// Categorical labels cannot be differenced without converting to a
    /// numeric or datetime dtype, matching pandas' fail-closed behavior.
    pub fn diff(&self, _periods: i64) -> Result<Vec<Option<i64>>, IndexError> {
        Err(IndexError::InvalidArgument(
            "Categorical has no 'diff' method; convert to a suitable dtype before calling diff"
                .to_owned(),
        ))
    }

    #[must_use]
    pub fn is_unique(&self) -> bool {
        let unique: FxHashSet<&String> = self.labels.iter().collect();
        unique.len() == self.labels.len()
    }

    #[must_use]
    pub fn has_duplicates(&self) -> bool {
        !self.is_unique()
    }

    #[must_use]
    pub fn is_monotonic_increasing(&self) -> bool {
        let codes = self.codes();
        codes.windows(2).all(|window| window[0] <= window[1])
    }

    #[must_use]
    pub fn is_monotonic(&self) -> bool {
        self.is_monotonic_increasing()
    }

    #[must_use]
    pub fn is_monotonic_decreasing(&self) -> bool {
        let codes = self.codes();
        codes.windows(2).all(|window| window[0] >= window[1])
    }

    #[must_use]
    pub fn nunique(&self) -> usize {
        self.labels.iter().collect::<FxHashSet<_>>().len()
    }

    #[must_use]
    pub fn ndim(&self) -> usize {
        1
    }

    pub fn item(&self) -> Result<String, IndexError> {
        if self.labels.len() == 1 {
            Ok(self.labels[0].clone())
        } else {
            Err(IndexError::InvalidArgument(format!(
                "item requires exactly one label, got {}",
                self.labels.len()
            )))
        }
    }

    #[must_use]
    pub fn is_(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }

    #[must_use]
    pub fn equals(&self, other: &Self) -> bool {
        self.labels == other.labels
            && self.categories == other.categories
            && self.ordered == other.ordered
    }

    #[must_use]
    pub fn identical(&self, other: &Self) -> bool {
        self.equals(other) && self.name == other.name
    }

    #[must_use]
    pub fn holds_integer(&self) -> bool {
        false
    }

    #[must_use]
    pub fn inferred_type(&self) -> &'static str {
        "categorical"
    }

    #[must_use]
    pub fn is_boolean(&self) -> bool {
        false
    }

    #[must_use]
    pub fn is_categorical(&self) -> bool {
        true
    }

    #[must_use]
    pub fn is_floating(&self) -> bool {
        false
    }

    #[must_use]
    pub fn is_integer(&self) -> bool {
        false
    }

    #[must_use]
    pub fn is_interval(&self) -> bool {
        false
    }

    #[must_use]
    pub fn is_numeric(&self) -> bool {
        false
    }

    #[must_use]
    pub fn is_object(&self) -> bool {
        false
    }

    /// First-occurrence category index for each category name, matching the
    /// semantics of `categories.iter().position(...)` but built once in O(k).
    /// `or_insert` keeps the first index if `categories` somehow has dupes.
    fn category_index_map(&self) -> FxHashMap<&str, usize> {
        let mut map: FxHashMap<&str, usize> = FxHashMap::default();
        for (i, cat) in self.categories.iter().enumerate() {
            map.entry(cat.as_str()).or_insert(i);
        }
        map
    }

    #[must_use]
    pub fn codes(&self) -> Vec<Option<usize>> {
        // O(n+k): hash category->index once instead of a linear
        // `categories.position` scan per label. First-occurrence index
        // preserved, so output is bit-identical.
        let map = self.category_index_map();
        self.labels
            .iter()
            .map(|label| map.get(label.as_str()).copied())
            .collect()
    }

    #[must_use]
    pub fn values(&self) -> Vec<String> {
        self.labels.clone()
    }

    #[must_use]
    pub fn to_list(&self) -> Vec<String> {
        self.labels.clone()
    }

    #[must_use]
    pub fn tolist(&self) -> Vec<String> {
        self.to_list()
    }

    #[must_use]
    pub fn to_numpy(&self) -> Vec<String> {
        self.labels.clone()
    }

    #[must_use]
    pub fn array(&self) -> Vec<String> {
        self.labels.clone()
    }

    #[must_use]
    pub fn to_index(&self) -> Index {
        Index::from_utf8(self.labels.clone()).set_names(self.name.as_deref())
    }

    /// Stringify each label, matching `pd.CategoricalIndex.format()`.
    /// Labels are already strings so this clones them.
    #[must_use]
    pub fn format(&self) -> Vec<String> {
        self.labels.clone()
    }

    /// Replace positions where `cond` is `false` with `other`, matching
    /// `pd.CategoricalIndex.where(cond, other)`. `other` must already be
    /// a member of the categories list.
    pub fn r#where(&self, cond: &[bool], other: &str) -> Result<Self, IndexError> {
        if cond.len() != self.labels.len() {
            return Err(IndexError::LengthMismatch {
                expected: self.labels.len(),
                actual: cond.len(),
                context: "where: cond length must match index length".to_owned(),
            });
        }
        if !self.categories.iter().any(|cat| cat == other) {
            return Err(IndexError::InvalidArgument(format!(
                "where: replacement {other:?} is not a category"
            )));
        }
        let labels: Vec<String> = self
            .labels
            .iter()
            .zip(cond.iter())
            .map(|(label, &keep)| {
                if keep {
                    label.clone()
                } else {
                    other.to_owned()
                }
            })
            .collect();
        Ok(Self {
            labels,
            categories: self.categories.clone(),
            ordered: self.ordered,
            name: self.name.clone(),
        })
    }

    /// Replace positions where `mask` is `true` with `value`, matching
    /// `pd.CategoricalIndex.putmask(mask, value)`.
    pub fn putmask(&self, mask: &[bool], value: &str) -> Result<Self, IndexError> {
        if mask.len() != self.labels.len() {
            return Err(IndexError::LengthMismatch {
                expected: self.labels.len(),
                actual: mask.len(),
                context: "putmask: mask length must match index length".to_owned(),
            });
        }
        if !self.categories.iter().any(|cat| cat == value) {
            return Err(IndexError::InvalidArgument(format!(
                "putmask: replacement {value:?} is not a category"
            )));
        }
        let labels: Vec<String> = self
            .labels
            .iter()
            .zip(mask.iter())
            .map(|(label, &replace)| {
                if replace {
                    value.to_owned()
                } else {
                    label.clone()
                }
            })
            .collect();
        Ok(Self {
            labels,
            categories: self.categories.clone(),
            ordered: self.ordered,
            name: self.name.clone(),
        })
    }

    /// Alias for [`isna`], matching `pd.CategoricalIndex.isnull()`.
    #[must_use]
    pub fn isnull(&self) -> Vec<bool> {
        self.isna()
    }

    /// Alias for [`notna`], matching `pd.CategoricalIndex.notnull()`.
    #[must_use]
    pub fn notnull(&self) -> Vec<bool> {
        self.notna()
    }

    /// Whether any label is missing, matching
    /// `pd.CategoricalIndex.hasnans`. Always `false` because the
    /// FrankenPandas storage carries only non-null Strings.
    #[must_use]
    pub fn hasnans(&self) -> bool {
        false
    }

    /// Drop missing positions, matching `pd.CategoricalIndex.dropna()`.
    /// Returns a clone because there are no missing labels to drop.
    #[must_use]
    pub fn dropna(&self) -> Self {
        self.clone()
    }

    /// Fill missing positions, matching `pd.CategoricalIndex.fillna(value)`.
    /// Returns a clone because there are no missing labels to fill;
    /// `value` is accepted for API parity but ignored.
    #[must_use]
    pub fn fillna(&self, _value: &str) -> Self {
        self.clone()
    }

    /// Mark the categorical as ordered, matching
    /// `pd.CategoricalIndex.as_ordered()`.
    #[must_use]
    pub fn as_ordered(&self) -> Self {
        let mut out = self.clone();
        out.ordered = true;
        out
    }

    /// Mark the categorical as unordered, matching
    /// `pd.CategoricalIndex.as_unordered()`.
    #[must_use]
    pub fn as_unordered(&self) -> Self {
        let mut out = self.clone();
        out.ordered = false;
        out
    }

    /// Extend the categories list with new entries, matching
    /// `pd.CategoricalIndex.add_categories(new)`. Rejects when any new
    /// category is already present.
    pub fn add_categories(&self, new: Vec<String>) -> Result<Self, IndexError> {
        // O(k_existing + k_new): hash the existing categories once instead of a
        // linear `categories.contains` scan per new entry. First clashing entry
        // (in `new` order) is still the one reported.
        let existing: FxHashSet<&str> = self.categories.iter().map(String::as_str).collect();
        for cat in &new {
            if existing.contains(cat.as_str()) {
                return Err(IndexError::InvalidArgument(format!(
                    "add_categories: {cat:?} is already a category"
                )));
            }
        }
        let mut categories = self.categories.clone();
        categories.extend(new);
        Ok(Self {
            labels: self.labels.clone(),
            categories,
            ordered: self.ordered,
            name: self.name.clone(),
        })
    }

    /// Drop categories from the list, matching
    /// `pd.CategoricalIndex.remove_categories(removals)`. Rejects when any
    /// removed category is still in use by a label (FrankenPandas does not
    /// yet carry NaN-labeled categoricals).
    pub fn remove_categories(&self, removals: &[String]) -> Result<Self, IndexError> {
        // Hash both the category set and the (large) label set once so the
        // per-removal validation is O(1) instead of two linear `contains`
        // scans — the `self.labels.contains` rescan was O(removals · n_labels).
        // Per-removal check order (not-a-category before in-use) is preserved,
        // so the first offending removal and its message are unchanged.
        let category_set: FxHashSet<&str> = self.categories.iter().map(String::as_str).collect();
        let label_set: FxHashSet<&str> = self.labels.iter().map(String::as_str).collect();
        for cat in removals {
            if !category_set.contains(cat.as_str()) {
                return Err(IndexError::InvalidArgument(format!(
                    "remove_categories: {cat:?} is not a category"
                )));
            }
            if label_set.contains(cat.as_str()) {
                return Err(IndexError::InvalidArgument(format!(
                    "remove_categories: {cat:?} is still in use by labels"
                )));
            }
        }
        let removals_set: FxHashSet<&String> = removals.iter().collect();
        let categories: Vec<String> = self
            .categories
            .iter()
            .filter(|cat| !removals_set.contains(cat))
            .cloned()
            .collect();
        Ok(Self {
            labels: self.labels.clone(),
            categories,
            ordered: self.ordered,
            name: self.name.clone(),
        })
    }

    /// Narrow categories to the set of labels actually present, matching
    /// `pd.CategoricalIndex.remove_unused_categories()`.
    #[must_use]
    pub fn remove_unused_categories(&self) -> Self {
        let used: FxHashSet<&String> = self.labels.iter().collect();
        let categories: Vec<String> = self
            .categories
            .iter()
            .filter(|cat| used.contains(cat))
            .cloned()
            .collect();
        Self {
            labels: self.labels.clone(),
            categories,
            ordered: self.ordered,
            name: self.name.clone(),
        }
    }

    /// Replace the categories list, matching
    /// `pd.CategoricalIndex.set_categories(new_categories)`. Rejects when
    /// any current label is missing from the new categories list.
    pub fn set_categories(&self, new_categories: Vec<String>) -> Result<Self, IndexError> {
        // O(n+k): hash the new category set once rather than scanning the new
        // categories Vec for every label. First label missing from the new
        // set (in label order) is still the one reported.
        let new_set: FxHashSet<&str> = new_categories.iter().map(String::as_str).collect();
        for label in &self.labels {
            if !new_set.contains(label.as_str()) {
                return Err(IndexError::InvalidArgument(format!(
                    "set_categories: label {label:?} is not in the new categories"
                )));
            }
        }
        Ok(Self {
            labels: self.labels.clone(),
            categories: new_categories,
            ordered: self.ordered,
            name: self.name.clone(),
        })
    }

    /// Rename categories pos-by-pos, matching
    /// `pd.CategoricalIndex.rename_categories(new)`. Rejects when the
    /// new list has a different length.
    pub fn rename_categories(&self, new: Vec<String>) -> Result<Self, IndexError> {
        if new.len() != self.categories.len() {
            return Err(IndexError::InvalidArgument(format!(
                "rename_categories: expected {} new names, got {}",
                self.categories.len(),
                new.len()
            )));
        }
        let mapping: std::collections::HashMap<&String, &String> =
            self.categories.iter().zip(new.iter()).collect();
        let labels: Vec<String> = self
            .labels
            .iter()
            .map(|label| (*mapping.get(label).expect("label is a category")).clone())
            .collect();
        Ok(Self {
            labels,
            categories: new,
            ordered: self.ordered,
            name: self.name.clone(),
        })
    }

    /// Reorder the categories list, matching
    /// `pd.CategoricalIndex.reorder_categories(new, ordered)`. Rejects
    /// when the new list is not a permutation of the existing categories.
    pub fn reorder_categories(&self, new: Vec<String>, ordered: bool) -> Result<Self, IndexError> {
        if new.len() != self.categories.len() {
            return Err(IndexError::InvalidArgument(format!(
                "reorder_categories: expected {} categories, got {}",
                self.categories.len(),
                new.len()
            )));
        }
        let existing: FxHashSet<&String> = self.categories.iter().collect();
        for cat in &new {
            if !existing.contains(cat) {
                return Err(IndexError::InvalidArgument(format!(
                    "reorder_categories: {cat:?} is not an existing category"
                )));
            }
        }
        let new_set: FxHashSet<&String> = new.iter().collect();
        if new_set.len() != new.len() {
            return Err(IndexError::InvalidArgument(
                "reorder_categories: new categories contain duplicates".to_owned(),
            ));
        }
        Ok(Self {
            labels: self.labels.clone(),
            categories: new,
            ordered,
            name: self.name.clone(),
        })
    }

    /// Convert to a flat [`Index`] of utf8 labels, matching
    /// `pd.CategoricalIndex.to_flat_index()`.
    #[must_use]
    pub fn to_flat_index(&self) -> Index {
        self.to_index()
    }

    /// String accessor for categorical string labels.
    #[must_use]
    pub fn r#str(&self) -> IndexStringAccessor<'_> {
        IndexStringAccessor::owned(self.to_flat_index())
    }

    /// One-column row materialization, matching `pd.CategoricalIndex.to_frame(index=False)`.
    #[must_use]
    pub fn to_frame(&self) -> Vec<Vec<IndexLabel>> {
        self.to_flat_index().to_frame()
    }

    /// Series-shaped materialization using category labels as both index and values.
    #[must_use]
    pub fn to_series(&self) -> Vec<(IndexLabel, IndexLabel)> {
        self.to_flat_index().to_series()
    }

    /// Whether any category label coerces to true.
    #[must_use]
    pub fn any(&self) -> bool {
        self.to_flat_index().any()
    }

    /// Whether all category labels coerce to true.
    #[must_use]
    pub fn all(&self) -> bool {
        self.to_flat_index().all()
    }

    /// Get labels for a level. CategoricalIndex is flat and only accepts level 0.
    pub fn get_level_values(&self, level: usize) -> Result<Index, IndexError> {
        self.to_flat_index().get_level_values(level)
    }

    /// Drop a level. CategoricalIndex is flat, so removing its only level is invalid.
    pub fn droplevel(&self, level: usize) -> Result<Index, IndexError> {
        self.to_flat_index().droplevel(level)
    }

    /// Group equal category labels into position buckets.
    #[must_use]
    pub fn groupby(&self) -> HashMap<IndexLabel, Vec<usize>> {
        self.to_flat_index().groupby()
    }

    /// Apply a function to each category label, returning a flat Index.
    #[must_use]
    pub fn map<F>(&self, func: F) -> Index
    where
        F: Fn(&IndexLabel) -> IndexLabel,
    {
        self.to_flat_index().map(func)
    }

    /// Cast category labels to a pandas dtype string, returning a flat Index.
    pub fn astype(&self, dtype: &str) -> Result<Index, IndexError> {
        self.to_flat_index().astype(dtype)
    }

    /// Nearest preceding-or-equal category label lookup.
    #[must_use]
    pub fn asof(&self, key: &IndexLabel) -> Option<IndexLabel> {
        self.to_flat_index().asof(key)
    }

    /// Locate nearest preceding-or-equal category positions for each target label.
    #[must_use]
    pub fn asof_locs(&self, where_index: &Index, mask: Option<&[bool]>) -> Vec<Option<usize>> {
        self.to_flat_index().asof_locs(where_index, mask)
    }

    /// Drop category labels, returning a flat Index.
    #[must_use]
    pub fn drop(&self, labels_to_drop: &[IndexLabel]) -> Index {
        self.to_flat_index().drop(labels_to_drop)
    }

    /// Join category labels with another flat Index.
    pub fn join(&self, other: &Index, how: &str) -> Result<Index, IndexError> {
        self.to_flat_index().join(other, how)
    }

    /// Sort category labels and return the positional sorter.
    #[must_use]
    pub fn sortlevel(&self) -> (Index, Vec<usize>) {
        self.to_flat_index().sortlevel()
    }

    /// Set the index name, matching `pd.CategoricalIndex.rename(name)`.
    #[must_use]
    pub fn rename(&self, name: &str) -> Self {
        self.set_name(name)
    }

    /// Returns a clone, matching `pd.CategoricalIndex.view()`.
    #[must_use]
    pub fn view(&self) -> Self {
        self.clone()
    }

    /// Identity transpose for a 1D index, matching
    /// `pd.CategoricalIndex.transpose()`.
    #[must_use]
    pub fn transpose(&self) -> Self {
        self.clone()
    }

    /// Alias for `transpose`, matching `pd.CategoricalIndex.T`.
    #[allow(non_snake_case)]
    #[must_use]
    pub fn T(&self) -> Self {
        self.transpose()
    }

    /// Flatten labels to a Vec<String>, matching
    /// `pd.CategoricalIndex.ravel()`.
    #[must_use]
    pub fn ravel(&self) -> Vec<String> {
        self.labels.clone()
    }

    /// Number of levels, matching `pd.CategoricalIndex.nlevels`. Always `1`.
    #[must_use]
    pub fn nlevels(&self) -> usize {
        1
    }

    /// Identity dtype-reinference, matching
    /// `pd.CategoricalIndex.infer_objects()`.
    #[must_use]
    pub fn infer_objects(&self) -> Self {
        self.clone()
    }

    /// Binary-search insertion position, matching
    /// `pd.CategoricalIndex.searchsorted(value, side)`. Forwarded through
    /// the underlying utf8 Index.
    pub fn searchsorted(&self, value: &str, side: &str) -> Result<usize, IndexError> {
        self.to_index()
            .searchsorted(&IndexLabel::Utf8(value.to_owned()), side)
    }

    /// Find positions of `[start, end]` for a label slice, matching
    /// `pd.CategoricalIndex.slice_locs(start, end)`. Requires labels to
    /// be sorted lexicographically (so the searchsorted result lines up
    /// with the slice boundary).
    pub fn slice_locs(&self, start: &str, end: &str) -> Result<(usize, usize), IndexError> {
        let labels_sorted = self.labels.windows(2).all(|w| w[0] <= w[1]);
        if !labels_sorted {
            return Err(IndexError::InvalidArgument(
                "slice_locs requires a CategoricalIndex with labels sorted lexicographically"
                    .to_owned(),
            ));
        }
        let left = self.searchsorted(start, "left")?;
        let right = self.searchsorted(end, "right")?;
        Ok((left, right))
    }

    /// Half-open positional range for a label slice, matching
    /// `pd.CategoricalIndex.slice_indexer(start, end)`.
    pub fn slice_indexer(
        &self,
        start: &str,
        end: &str,
    ) -> Result<std::ops::Range<usize>, IndexError> {
        let (l, r) = self.slice_locs(start, end)?;
        Ok(l..r)
    }

    fn set_op_via_string<F>(&self, other: &Self, op: F) -> Self
    where
        F: FnOnce(Vec<&String>, Vec<&String>) -> Vec<String>,
    {
        let labels = op(self.labels.iter().collect(), other.labels.iter().collect());
        // Dedup the union of categories with a seen-set instead of an O(k)
        // `Vec::contains` per label (O(n·k) for high-cardinality categoricals).
        // `seen` borrows self.categories + labels (both stable) — never the
        // growing `categories` Vec — so a label is pushed iff it is neither an
        // existing category nor already pushed this pass: identical first-seen
        // order and dedup to the linear scan.
        let mut categories: Vec<String> = self.categories.clone();
        let mut seen: FxHashSet<&String> = self.categories.iter().collect();
        for label in &labels {
            if seen.insert(label) {
                categories.push(label.clone());
            }
        }
        Self {
            labels,
            categories,
            ordered: self.ordered,
            name: if self.name == other.name {
                self.name.clone()
            } else {
                None
            },
        }
    }

    /// Labels in both indexes (first-seen order from self), matching
    /// `pd.CategoricalIndex.intersection(other)`.
    #[must_use]
    pub fn intersection(&self, other: &Self) -> Self {
        self.set_op_via_string(other, |left, right| {
            let right_set: FxHashSet<&&String> = right.iter().collect();
            let mut seen = FxHashSet::<&String>::default();
            left.into_iter()
                .filter(|label| right_set.contains(label) && seen.insert(label))
                .cloned()
                .collect()
        })
    }

    /// Self labels then other labels not seen, matching
    /// `pd.CategoricalIndex.union(other)`.
    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        self.set_op_via_string(other, |left, right| {
            let mut seen = FxHashSet::<&String>::default();
            left.into_iter()
                .chain(right)
                .filter(|label| seen.insert(label))
                .cloned()
                .collect()
        })
    }

    /// Labels in either but not both, matching
    /// `pd.CategoricalIndex.symmetric_difference(other)`.
    #[must_use]
    pub fn symmetric_difference(&self, other: &Self) -> Self {
        self.set_op_via_string(other, |left, right| {
            let left_set: FxHashSet<&&String> = left.iter().collect();
            let right_set: FxHashSet<&&String> = right.iter().collect();
            let mut seen = FxHashSet::<&String>::default();
            let mut out = Vec::<String>::new();
            for label in &left {
                if !right_set.contains(label) && seen.insert(*label) {
                    out.push((*label).clone());
                }
            }
            for label in &right {
                if !left_set.contains(label) && seen.insert(*label) {
                    out.push((*label).clone());
                }
            }
            out
        })
    }

    /// Self labels not in other, matching
    /// `pd.CategoricalIndex.difference(other)`.
    #[must_use]
    pub fn difference(&self, other: &Self) -> Self {
        // Per br-frankenpandas-6r1lq: difference preserves self.name (not
        // shared_name like set_op_via_string applies for union/intersection).
        let mut out = self.set_op_via_string(other, |left, right| {
            let right_set: FxHashSet<&&String> = right.iter().collect();
            let mut seen = FxHashSet::<&String>::default();
            left.into_iter()
                .filter(|label| !right_set.contains(label) && seen.insert(label))
                .cloned()
                .collect()
        });
        out.name = self.name.clone();
        out
    }

    /// Sort labels ascending, matching `pd.CategoricalIndex.sort_values()`.
    /// `ordered=true` sorts by category position; `ordered=false` sorts
    /// lexicographically. Categories list and ordered flag are preserved.
    #[must_use]
    pub fn sort_values(&self) -> Self {
        let positions = self.argsort();
        let labels: Vec<String> = positions.iter().map(|&p| self.labels[p].clone()).collect();
        Self {
            labels,
            categories: self.categories.clone(),
            ordered: self.ordered,
            name: self.name.clone(),
        }
    }

    /// Alias for `sort_values`, matching `pd.CategoricalIndex.sort()`.
    #[must_use]
    pub fn sort(&self) -> Self {
        self.sort_values()
    }

    /// Positions that would sort labels ascending, matching
    /// `pd.CategoricalIndex.argsort()`.
    ///
    /// pandas sorts a Categorical by its **category codes** — the position of
    /// each label within `categories` — for both ordered and unordered
    /// categoricals (`Categorical._values_for_argsort` returns `self.codes`),
    /// NOT lexicographically by the label text. So categories `[b, a, c]` sort
    /// before-`a` because `b` has code 0. The sort is stable, so equal-code
    /// ties keep their original order. CategoricalIndex labels are non-null, so
    /// every label resolves to a code. When the category order happens to be
    /// lexicographic this is identical to the old text sort; only
    /// non-lexicographic category orders are corrected.
    #[must_use]
    pub fn argsort(&self) -> Vec<usize> {
        let map = self.category_index_map();
        let mut positions: Vec<usize> = (0..self.labels.len()).collect();
        positions.sort_by_key(|&i| {
            map.get(self.labels[i].as_str())
                .copied()
                .unwrap_or(usize::MAX)
        });
        positions
    }

    /// Concatenate with another CategoricalIndex, matching
    /// `pd.CategoricalIndex.append(other)`. Categories merge
    /// (other-only categories are appended) and the index name is
    /// preserved when both operands share it.
    #[must_use]
    pub fn append(&self, other: &Self) -> Self {
        let mut labels = self.labels.clone();
        labels.extend_from_slice(&other.labels);
        // Union categories with a seen-set, not O(k) `Vec::contains` per entry
        // (see set_op_via_string). `seen` borrows self/other categories, never
        // the growing `categories` Vec; identical first-seen order + dedup.
        let mut categories = self.categories.clone();
        let mut seen: FxHashSet<&String> = self.categories.iter().collect();
        for cat in &other.categories {
            if seen.insert(cat) {
                categories.push(cat.clone());
            }
        }
        let name = if self.name == other.name {
            self.name.clone()
        } else {
            None
        };
        Self {
            labels,
            categories,
            ordered: self.ordered && other.ordered,
            name,
        }
    }

    /// Remove the label at the given position, matching
    /// `pd.CategoricalIndex.delete(loc)`. OOB raises.
    pub fn delete(&self, loc: usize) -> Result<Self, IndexError> {
        if loc >= self.labels.len() {
            return Err(IndexError::OutOfBounds {
                position: loc,
                length: self.labels.len(),
            });
        }
        let mut labels = self.labels.clone();
        labels.remove(loc);
        Ok(Self {
            labels,
            categories: self.categories.clone(),
            ordered: self.ordered,
            name: self.name.clone(),
        })
    }

    /// Insert `value` at position `loc`, matching
    /// `pd.CategoricalIndex.insert(loc, value)`. The value must be a
    /// member of the categories list; OOB and not-a-category raise.
    pub fn insert(&self, loc: usize, value: &str) -> Result<Self, IndexError> {
        if loc > self.labels.len() {
            return Err(IndexError::OutOfBounds {
                position: loc,
                length: self.labels.len(),
            });
        }
        if !self.categories.iter().any(|cat| cat == value) {
            return Err(IndexError::InvalidArgument(format!(
                "insert: {value:?} is not a category"
            )));
        }
        let mut labels = self.labels.clone();
        labels.insert(loc, value.to_owned());
        Ok(Self {
            labels,
            categories: self.categories.clone(),
            ordered: self.ordered,
            name: self.name.clone(),
        })
    }

    /// Repeat each label `repeats` times, matching
    /// `pd.CategoricalIndex.repeat(repeats)`.
    #[must_use]
    pub fn repeat(&self, repeats: usize) -> Self {
        let mut labels = Vec::with_capacity(self.labels.len() * repeats);
        for label in &self.labels {
            for _ in 0..repeats {
                labels.push(label.clone());
            }
        }
        Self {
            labels,
            categories: self.categories.clone(),
            ordered: self.ordered,
            name: self.name.clone(),
        }
    }

    /// Pick labels at the given positions, matching
    /// `pd.CategoricalIndex.take(positions)`. Out-of-bounds positions
    /// raise [`IndexError::OutOfBounds`].
    pub fn take(&self, positions: &[usize]) -> Result<Self, IndexError> {
        for &p in positions {
            if p >= self.labels.len() {
                return Err(IndexError::OutOfBounds {
                    position: p,
                    length: self.labels.len(),
                });
            }
        }
        let labels: Vec<String> = positions.iter().map(|&p| self.labels[p].clone()).collect();
        Ok(Self {
            labels,
            categories: self.categories.clone(),
            ordered: self.ordered,
            name: self.name.clone(),
        })
    }

    /// Per-position membership mask, matching
    /// `pd.CategoricalIndex.isin(values)`.
    #[must_use]
    pub fn isin(&self, values: &[String]) -> Vec<bool> {
        let needle: FxHashSet<&String> = values.iter().collect();
        self.labels.iter().map(|l| needle.contains(l)).collect()
    }

    /// Locate every position matching each target, matching
    /// `pd.CategoricalIndex.get_indexer_non_unique(targets)`.
    #[must_use]
    pub fn get_indexer_non_unique(&self, targets: &[String]) -> (Vec<isize>, Vec<usize>) {
        let mut by_value = FxHashMap::<&String, Vec<usize>>::default();
        for (i, label) in self.labels.iter().enumerate() {
            by_value.entry(label).or_default().push(i);
        }
        let mut positions = Vec::<isize>::new();
        let mut missing = Vec::<usize>::new();
        for (idx, target) in targets.iter().enumerate() {
            if let Some(matches) = by_value.get(target) {
                positions.extend(
                    matches
                        .iter()
                        .map(|p| isize::try_from(*p).unwrap_or(isize::MAX)),
                );
            } else {
                positions.push(-1);
                missing.push(idx);
            }
        }
        (positions, missing)
    }

    /// Locate each label in `targets`, matching
    /// `pd.CategoricalIndex.get_indexer(targets)`.
    #[must_use]
    pub fn get_indexer(&self, targets: &[String]) -> Vec<isize> {
        let mut positions = FxHashMap::<&String, isize>::default();
        for (i, label) in self.labels.iter().enumerate() {
            positions
                .entry(label)
                .or_insert_with(|| isize::try_from(i).unwrap_or(isize::MAX));
        }
        targets
            .iter()
            .map(|t| positions.get(t).copied().unwrap_or(-1))
            .collect()
    }

    /// Alias for [`get_indexer`], matching
    /// `pd.CategoricalIndex.get_indexer_for(targets)`.
    #[must_use]
    pub fn get_indexer_for(&self, targets: &[String]) -> Vec<isize> {
        self.get_indexer(targets)
    }

    /// First position of `value`, matching
    /// `pd.CategoricalIndex.get_loc(value)`.
    pub fn get_loc(&self, value: &str) -> Result<usize, IndexError> {
        self.labels.iter().position(|l| l == value).ok_or_else(|| {
            IndexError::InvalidArgument(format!("get_loc: {value:?} not in CategoricalIndex"))
        })
    }

    /// Position of the maximum label, matching
    /// `pd.CategoricalIndex.argmax()`. ordered=true uses category
    /// position; unordered uses lexicographic ordering. Empty raises
    /// pandas-style "attempt to get argmax of an empty sequence".
    pub fn argmax(&self) -> Result<usize, IndexError> {
        if self.labels.is_empty() {
            return Err(IndexError::InvalidArgument(
                "attempt to get argmax of an empty sequence".to_owned(),
            ));
        }
        let mut best = 0;
        if self.ordered {
            let map = self.category_index_map();
            let position = |label: &String| map.get(label.as_str()).copied().unwrap_or(0);
            for i in 1..self.labels.len() {
                if position(&self.labels[i]) > position(&self.labels[best]) {
                    best = i;
                }
            }
        } else {
            for i in 1..self.labels.len() {
                if self.labels[i] > self.labels[best] {
                    best = i;
                }
            }
        }
        Ok(best)
    }

    /// Position of the minimum label, matching
    /// `pd.CategoricalIndex.argmin()`. Same ordering rules as argmax.
    pub fn argmin(&self) -> Result<usize, IndexError> {
        if self.labels.is_empty() {
            return Err(IndexError::InvalidArgument(
                "attempt to get argmin of an empty sequence".to_owned(),
            ));
        }
        let mut best = 0;
        if self.ordered {
            let map = self.category_index_map();
            let position = |label: &String| map.get(label.as_str()).copied().unwrap_or(usize::MAX);
            for i in 1..self.labels.len() {
                if position(&self.labels[i]) < position(&self.labels[best]) {
                    best = i;
                }
            }
        } else {
            for i in 1..self.labels.len() {
                if self.labels[i] < self.labels[best] {
                    best = i;
                }
            }
        }
        Ok(best)
    }

    /// Smallest label in category order when ordered, lexicographic when
    /// unordered, matching `pd.CategoricalIndex.min()`. Empty returns
    /// `None`.
    #[must_use]
    pub fn min(&self) -> Option<&str> {
        if self.labels.is_empty() {
            return None;
        }
        if self.ordered {
            // Compare by category position (hashed once, O(n+k)).
            let map = self.category_index_map();
            let position = |label: &String| map.get(label.as_str()).copied().unwrap_or(usize::MAX);
            self.labels
                .iter()
                .min_by_key(|label| position(label))
                .map(String::as_str)
        } else {
            self.labels.iter().min().map(String::as_str)
        }
    }

    /// Largest label, matching `pd.CategoricalIndex.max()`.
    #[must_use]
    pub fn max(&self) -> Option<&str> {
        if self.labels.is_empty() {
            return None;
        }
        if self.ordered {
            let map = self.category_index_map();
            let position = |label: &String| map.get(label.as_str()).copied().unwrap_or(0);
            self.labels
                .iter()
                .max_by_key(|label| position(label))
                .map(String::as_str)
        } else {
            self.labels.iter().max().map(String::as_str)
        }
    }

    /// First-seen unique labels, matching `pd.CategoricalIndex.unique()`.
    /// Categories are preserved (not narrowed to seen labels) and the
    /// ordered flag rolls through. The result keeps the index name.
    #[must_use]
    pub fn unique(&self) -> Self {
        let mut seen = FxHashSet::<&String>::default();
        let mut uniques = Vec::<String>::new();
        for label in &self.labels {
            if seen.insert(label) {
                uniques.push(label.clone());
            }
        }
        Self {
            labels: uniques,
            categories: self.categories.clone(),
            ordered: self.ordered,
            name: self.name.clone(),
        }
    }

    /// Per-position duplicate mask, matching
    /// `pd.CategoricalIndex.duplicated(keep)`.
    #[must_use]
    pub fn duplicated(&self, keep: DuplicateKeep) -> Vec<bool> {
        self.to_index().duplicated(keep)
    }

    /// Drop duplicate labels (keep first), matching
    /// `pd.CategoricalIndex.drop_duplicates()`. Categories and ordered
    /// flag are preserved.
    #[must_use]
    pub fn drop_duplicates(&self) -> Self {
        self.unique()
    }

    /// Value counts, matching `pd.CategoricalIndex.value_counts()`.
    /// CategoricalIndex labels are non-null so the total equals `len()`.
    #[must_use]
    pub fn value_counts(&self) -> Vec<(String, usize)> {
        let mut order = Vec::<&String>::new();
        let mut counts = FxHashMap::<&String, usize>::default();
        for label in &self.labels {
            let entry = counts.entry(label).or_insert_with(|| {
                order.push(label);
                0
            });
            *entry += 1;
        }
        let mut pairs: Vec<(String, usize)> =
            order.iter().map(|s| ((*s).clone(), counts[*s])).collect();
        // Pandas sorts descending by count for value_counts.
        pairs.sort_by_key(|entry| std::cmp::Reverse(entry.1));
        pairs
    }

    /// Factorize, matching `pd.CategoricalIndex.factorize()`. Returns
    /// `(codes, uniques)` where `uniques` is a CategoricalIndex with
    /// the same categories list.
    #[must_use]
    pub fn factorize(&self) -> (Vec<isize>, Self) {
        let mut positions = FxHashMap::<&String, isize>::default();
        let mut uniques = Vec::<String>::new();
        let mut codes = Vec::with_capacity(self.labels.len());
        for label in &self.labels {
            if let Some(code) = positions.get(label) {
                codes.push(*code);
            } else {
                let code = isize::try_from(uniques.len()).unwrap_or(isize::MAX);
                positions.insert(label, code);
                uniques.push(label.clone());
                codes.push(code);
            }
        }
        let unique_index = Self {
            labels: uniques,
            categories: self.categories.clone(),
            ordered: self.ordered,
            name: self.name.clone(),
        };
        (codes, unique_index)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlignmentPlan {
    pub union_index: Index,
    pub left_positions: Vec<Option<usize>>,
    pub right_positions: Vec<Option<usize>>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum IndexError {
    #[error("alignment vectors must have equal lengths")]
    InvalidAlignmentVectors,
    #[error("position {position} out of bounds for length {length}")]
    OutOfBounds { position: usize, length: usize },
    #[error("length mismatch: expected {expected}, got {actual} ({context})")]
    LengthMismatch {
        expected: usize,
        actual: usize,
        context: String,
    },
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
}

/// Alignment mode for index-level join semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignMode {
    /// Only labels present in both indexes.
    Inner,
    /// All left labels; right fills with None for missing.
    Left,
    /// All right labels; left fills with None for missing.
    Right,
    /// All labels from both indexes (union). Default for arithmetic.
    Outer,
}

fn index_position_groups(index: &Index) -> FxHashMap<IndexLabel, Vec<usize>> {
    let mut groups: FxHashMap<IndexLabel, Vec<usize>> = FxHashMap::default();
    for (pos, label) in index.labels().iter().enumerate() {
        groups.entry(label.clone()).or_default().push(pos);
    }
    groups
}

fn align_non_unique(left: &Index, right: &Index, mode: AlignMode) -> AlignmentPlan {
    let left_groups = index_position_groups(left);
    let right_groups = index_position_groups(right);

    let mut out_labels = Vec::new();
    let mut left_positions = Vec::new();
    let mut right_positions = Vec::new();

    match mode {
        AlignMode::Inner => {
            for (left_pos, label) in left.labels().iter().enumerate() {
                if let Some(right_hits) = right_groups.get(label) {
                    for &right_pos in right_hits {
                        out_labels.push(label.clone());
                        left_positions.push(Some(left_pos));
                        right_positions.push(Some(right_pos));
                    }
                }
            }
        }
        AlignMode::Left => {
            for (left_pos, label) in left.labels().iter().enumerate() {
                match right_groups.get(label) {
                    Some(right_hits) if !right_hits.is_empty() => {
                        for &right_pos in right_hits {
                            out_labels.push(label.clone());
                            left_positions.push(Some(left_pos));
                            right_positions.push(Some(right_pos));
                        }
                    }
                    _ => {
                        out_labels.push(label.clone());
                        left_positions.push(Some(left_pos));
                        right_positions.push(None);
                    }
                }
            }
        }
        AlignMode::Right => {
            for (right_pos, label) in right.labels().iter().enumerate() {
                match left_groups.get(label) {
                    Some(left_hits) if !left_hits.is_empty() => {
                        for &left_pos in left_hits {
                            out_labels.push(label.clone());
                            left_positions.push(Some(left_pos));
                            right_positions.push(Some(right_pos));
                        }
                    }
                    _ => {
                        out_labels.push(label.clone());
                        left_positions.push(None);
                        right_positions.push(Some(right_pos));
                    }
                }
            }
        }
        AlignMode::Outer => {
            for (left_pos, label) in left.labels().iter().enumerate() {
                match right_groups.get(label) {
                    Some(right_hits) if !right_hits.is_empty() => {
                        for &right_pos in right_hits {
                            out_labels.push(label.clone());
                            left_positions.push(Some(left_pos));
                            right_positions.push(Some(right_pos));
                        }
                    }
                    _ => {
                        out_labels.push(label.clone());
                        left_positions.push(Some(left_pos));
                        right_positions.push(None);
                    }
                }
            }

            for (right_pos, label) in right.labels().iter().enumerate() {
                if !left_groups.contains_key(label) {
                    out_labels.push(label.clone());
                    left_positions.push(None);
                    right_positions.push(Some(right_pos));
                }
            }
        }
    }

    let mut union_index = Index::new(out_labels);
    match mode {
        AlignMode::Left => {
            union_index.name = left.name.clone();
        }
        AlignMode::Right => {
            union_index.name = right.name.clone();
        }
        AlignMode::Inner | AlignMode::Outer => {}
    }

    AlignmentPlan {
        union_index,
        left_positions,
        right_positions,
    }
}

/// Align two indexes using the specified join mode.
///
/// Returns an `AlignmentPlan` whose `union_index` contains the output index
/// (which may be an intersection, left-only, right-only, or union depending on mode).
pub fn align(left: &Index, right: &Index, mode: AlignMode) -> AlignmentPlan {
    if left.has_duplicates() || right.has_duplicates() {
        return align_non_unique(left, right, mode);
    }

    match mode {
        AlignMode::Inner => align_inner(left, right),
        AlignMode::Left => align_left(left, right),
        AlignMode::Right => {
            let plan = align_left(right, left);
            AlignmentPlan {
                union_index: plan.union_index,
                left_positions: plan.right_positions,
                right_positions: plan.left_positions,
            }
        }
        AlignMode::Outer => align_union(left, right),
    }
}

/// Inner alignment: only labels present in both indexes.
///
/// For non-unique labels, emits cartesian matches preserving left order.
pub fn align_inner(left: &Index, right: &Index) -> AlignmentPlan {
    if left.has_duplicates() || right.has_duplicates() {
        return align_non_unique(left, right, AlignMode::Inner);
    }

    let right_map = right.position_map_first_ref();

    let mut output_labels = Vec::new();
    let mut left_positions = Vec::new();
    let mut right_positions = Vec::new();

    for (left_pos, label) in left.labels.iter().enumerate() {
        if let Some(&right_pos) = right_map.get(label) {
            output_labels.push(label.clone());
            left_positions.push(Some(left_pos));
            right_positions.push(Some(right_pos));
        }
    }

    // Per br-frankenpandas-m2i5n: pandas inner alignment preserves the
    // shared index name (preserved when both operands agree, None when
    // they differ). Mirrors align_non_unique handling.
    let shared_name = if left.name() == right.name() {
        left.name().map(str::to_owned)
    } else {
        None
    };
    let mut union_index = Index::new(output_labels);
    union_index.name = shared_name;
    AlignmentPlan {
        union_index,
        left_positions,
        right_positions,
    }
}

/// Left alignment: all left labels preserved, right fills with None for missing.
pub fn align_left(left: &Index, right: &Index) -> AlignmentPlan {
    if left.has_duplicates() || right.has_duplicates() {
        return align_non_unique(left, right, AlignMode::Left);
    }

    let right_map = right.position_map_first_ref();

    let mut left_positions = Vec::with_capacity(left.len());
    let mut right_positions = Vec::with_capacity(left.len());

    for (left_pos, label) in left.labels.iter().enumerate() {
        left_positions.push(Some(left_pos));
        right_positions.push(right_map.get(label).copied());
    }

    AlignmentPlan {
        union_index: left.clone(),
        left_positions,
        right_positions,
    }
}

pub fn align_union(left: &Index, right: &Index) -> AlignmentPlan {
    if left.has_duplicates() || right.has_duplicates() {
        return align_non_unique(left, right, AlignMode::Outer);
    }

    let left_positions_map = left.position_map_first_ref();
    let right_positions_map = right.position_map_first_ref();

    let mut union_labels = Vec::with_capacity(left.labels.len() + right.labels.len());
    union_labels.extend(left.labels.iter().cloned());
    for label in &right.labels {
        if !left_positions_map.contains_key(&label) {
            union_labels.push(label.clone());
        }
    }

    let left_positions = union_labels
        .iter()
        .map(|label| left_positions_map.get(&label).copied())
        .collect();

    let right_positions = union_labels
        .iter()
        .map(|label| right_positions_map.get(&label).copied())
        .collect();

    // Per br-frankenpandas-r4k11: pandas outer alignment preserves the
    // shared index name. Mirrors align_inner / align_non_unique handling.
    let shared_name = if left.name() == right.name() {
        left.name().map(str::to_owned)
    } else {
        None
    };
    let mut union_index = Index::new(union_labels);
    union_index.name = shared_name;
    AlignmentPlan {
        union_index,
        left_positions,
        right_positions,
    }
}

pub fn validate_alignment_plan(plan: &AlignmentPlan) -> Result<(), IndexError> {
    if plan.left_positions.len() != plan.right_positions.len()
        || plan.left_positions.len() != plan.union_index.len()
    {
        return Err(IndexError::InvalidAlignmentVectors);
    }

    Ok(())
}

// ── AG-11: Leapfrog Triejoin for Multi-Way Index Alignment ─────────────

/// Result of multi-way alignment: a union index plus per-input position vectors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiAlignmentPlan {
    pub union_index: Index,
    pub positions: Vec<Vec<Option<usize>>>,
}

/// K-way merge union of multiple sorted iterators.
///
/// Produces a sorted, deduplicated index containing all labels from all inputs.
/// Each input is sorted internally before merging. Uses a min-heap for O(N log K)
/// performance where N = total labels and K = number of indexes.
pub fn leapfrog_union(indexes: &[&Index]) -> Index {
    if indexes.is_empty() {
        return Index::new(Vec::new());
    }
    if indexes.len() == 1 {
        return indexes[0].unique().sort_values();
    }

    // Sort and dedup each input
    let sorted: Vec<Vec<&IndexLabel>> = indexes
        .iter()
        .map(|idx| {
            let mut labels: Vec<&IndexLabel> = idx.labels().iter().collect();
            labels.sort();
            labels.dedup();
            labels
        })
        .collect();

    // Initialize min-heap: (label, iter_index, position_in_iter)
    let mut heap = std::collections::BinaryHeap::new();
    for (i, iter) in sorted.iter().enumerate() {
        if !iter.is_empty() {
            heap.push(std::cmp::Reverse((iter[0].clone(), i, 0_usize)));
        }
    }

    let total: usize = sorted.iter().map(|s| s.len()).sum();
    let mut result = Vec::with_capacity(total);

    while let Some(std::cmp::Reverse((label, iter_idx, pos))) = heap.pop() {
        // Deduplicate: only push if different from last
        if result.last() != Some(&label) {
            result.push(label);
        }

        let next_pos = pos + 1;
        if next_pos < sorted[iter_idx].len() {
            heap.push(std::cmp::Reverse((
                sorted[iter_idx][next_pos].clone(),
                iter_idx,
                next_pos,
            )));
        }
    }

    Index::new(result)
}

/// Leapfrog intersection: labels present in ALL input indexes.
///
/// Classic leapfrog algorithm on sorted iterators. For each position,
/// advance the smallest iterator to seek the maximum. When all iterators
/// agree, emit the label.
pub fn leapfrog_intersection(indexes: &[&Index]) -> Index {
    if indexes.is_empty() {
        return Index::new(Vec::new());
    }
    if indexes.len() == 1 {
        return indexes[0].unique().sort_values();
    }

    // Sort and dedup each input
    let sorted: Vec<Vec<&IndexLabel>> = indexes
        .iter()
        .map(|idx| {
            let mut labels: Vec<&IndexLabel> = idx.labels().iter().collect();
            labels.sort();
            labels.dedup();
            labels
        })
        .collect();

    // Cursors into each sorted iterator
    let k = sorted.len();
    let mut cursors: Vec<usize> = vec![0; k];
    let mut result = Vec::new();

    'outer: loop {
        // Check if any iterator is exhausted
        for i in 0..k {
            if cursors[i] >= sorted[i].len() {
                break 'outer;
            }
        }

        // Find the max label across all cursors
        let mut max_label = sorted[0][cursors[0]];
        for i in 1..k {
            if sorted[i][cursors[i]] > max_label {
                max_label = sorted[i][cursors[i]];
            }
        }

        // Advance all cursors to at least max_label
        let mut all_equal = true;
        for i in 0..k {
            // Binary search for max_label in sorted[i] starting from cursors[i]
            let remaining = &sorted[i][cursors[i]..];
            match remaining.binary_search(&max_label) {
                Ok(offset) => {
                    cursors[i] += offset;
                }
                Err(offset) => {
                    cursors[i] += offset;
                    all_equal = false;
                }
            }
            if cursors[i] >= sorted[i].len() {
                break 'outer;
            }
        }

        if all_equal {
            // All iterators point to the same label
            result.push(max_label.clone());
            for cursor in &mut cursors {
                *cursor += 1;
            }
        }
        // If not all equal, the loop continues with updated cursors
    }

    Index::new(result)
}

/// Multi-way alignment: union all indexes, then compute position vectors.
///
/// This is the AGM-bound-optimal replacement for iterative pairwise `align_union`.
/// For N indexes, produces a single sorted union index and N position vectors
/// mapping each union label to its original position in each input.
pub fn multi_way_align(indexes: &[&Index]) -> MultiAlignmentPlan {
    if indexes.is_empty() {
        return MultiAlignmentPlan {
            union_index: Index::new(Vec::new()),
            positions: Vec::new(),
        };
    }

    // Preserve pandas-style union ordering: start with the first index's labels,
    // then append unseen labels from subsequent indexes in encounter order.
    // This matches iterative align_union(sort=False) semantics while avoiding
    // the O(N*K) pairwise alignment cascade.
    // Borrow labels into the membership set (no clone per label) and clone only
    // the first-seen ones into the output. The prior version cloned EVERY label
    // into an owned HashSet<IndexLabel> (even duplicates) — clone-bound. Borrowed
    // keys + FxHashSet leave only the unique-label output clones. The borrow is
    // valid: every &IndexLabel comes from `indexes`, which outlives this scan.
    let mut seen: FxHashSet<&IndexLabel> = FxHashSet::with_capacity_and_hasher(
        indexes.iter().map(|idx| idx.labels().len()).sum(),
        Default::default(),
    );
    let mut union_labels: Vec<IndexLabel> = Vec::new();
    for idx in indexes {
        for label in idx.labels() {
            if seen.insert(label) {
                union_labels.push(label.clone());
            }
        }
    }
    // Per br-frankenpandas-nrhjq: pandas multi-index union sets name to
    // the shared name across all inputs (= None if any differ).
    let first_name = indexes
        .first()
        .and_then(|idx| idx.name())
        .map(str::to_owned);
    let shared_name = if indexes
        .iter()
        .all(|idx| idx.name() == first_name.as_deref())
    {
        first_name
    } else {
        None
    };
    let mut union = Index::new(union_labels);
    union.name = shared_name;

    // Build position maps for each input
    let maps: Vec<FxHashMap<&IndexLabel, usize>> = indexes
        .iter()
        .map(|idx| idx.position_map_first_ref())
        .collect();

    let positions: Vec<Vec<Option<usize>>> = maps
        .iter()
        .map(|map| {
            union
                .labels
                .iter()
                .map(|label| map.get(label).copied())
                .collect()
        })
        .collect();

    MultiAlignmentPlan {
        union_index: union,
        positions,
    }
}

// ── TimedeltaIndex helpers ──────────────────────────────────────────────

/// Error for timedelta_range parameter combinations.
#[derive(Debug, Clone, Error)]
pub enum TimedeltaRangeError {
    #[error("must specify at least two of start, end, periods")]
    InsufficientParams,
    #[error("must specify no more than two of start, end, periods")]
    TooManyParams,
    #[error("freq must be positive")]
    NonPositiveFreq,
    #[error("cannot compute range: end < start with positive freq")]
    InvalidRange,
}

/// Create a TimedeltaIndex with evenly spaced values.
///
/// Analogous to `pd.timedelta_range()`. Must specify exactly two of:
/// start, end, periods. Frequency defaults to 1 day (86_400_000_000_000 ns).
///
/// # Examples
/// ```
/// use fp_index::timedelta_range;
/// use fp_types::Timedelta;
///
/// let idx = timedelta_range(
///     Some(Timedelta::NANOS_PER_DAY),
///     None,
///     Some(3),
///     Timedelta::NANOS_PER_DAY,
///     None,
/// ).unwrap();
/// assert_eq!(idx.len(), 3);
/// ```
pub fn timedelta_range(
    start: Option<i64>,
    end: Option<i64>,
    periods: Option<usize>,
    freq: i64,
    name: Option<&str>,
) -> Result<Index, TimedeltaRangeError> {
    if freq <= 0 {
        return Err(TimedeltaRangeError::NonPositiveFreq);
    }

    let (start_ns, count) = match (start, end, periods) {
        (Some(s), Some(e), None) => {
            if e < s {
                return Err(TimedeltaRangeError::InvalidRange);
            }
            let n = ((e - s) / freq + 1) as usize;
            (s, n)
        }
        (Some(s), None, Some(p)) => (s, p),
        (None, Some(e), Some(p)) => {
            let s = e - (p.saturating_sub(1) as i64) * freq;
            (s, p)
        }
        (Some(_), Some(_), Some(_)) => return Err(TimedeltaRangeError::TooManyParams),
        _ => return Err(TimedeltaRangeError::InsufficientParams),
    };

    let nanos: Vec<i64> = (0..count).map(|i| start_ns + (i as i64) * freq).collect();
    let mut idx = Index::from_timedelta64(nanos);
    if let Some(n) = name {
        idx = idx.set_name(n);
    }
    Ok(idx)
}

// ── DatetimeIndex helpers ───────────────────────────────────────────────

/// Error for date_range parameter combinations.
#[derive(Debug, Clone, Error)]
pub enum DateRangeError {
    #[error("must specify at least two of start, end, periods")]
    InsufficientParams,
    #[error("need at least 3 dates to infer frequency")]
    InsufficientDates,
    #[error("must specify no more than two of start, end, periods")]
    TooManyParams,
    #[error("freq must be positive")]
    NonPositiveFreq,
    #[error("cannot compute range: end < start with positive freq")]
    InvalidRange,
    #[error("invalid datetime string: {0}")]
    ParseError(String),
}

/// Parse an ISO 8601 datetime string to nanoseconds since epoch.
fn parse_datetime_to_nanos(s: &str) -> Result<i64, DateRangeError> {
    use chrono::NaiveDateTime;

    let trimmed = s.trim();

    // Try full datetime format
    if let Ok(dt) = NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S") {
        return datetime_to_nanos(dt);
    }
    if let Ok(dt) = NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S") {
        return datetime_to_nanos(dt);
    }

    // Try date-only format (midnight)
    if let Ok(date) = chrono::NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        let dt = date
            .and_hms_opt(0, 0, 0)
            .ok_or(DateRangeError::InvalidRange)?;
        return datetime_to_nanos(dt);
    }

    Err(DateRangeError::ParseError(trimmed.to_owned()))
}

fn datetime_to_nanos(dt: chrono::NaiveDateTime) -> Result<i64, DateRangeError> {
    dt.and_utc()
        .timestamp_nanos_opt()
        .ok_or(DateRangeError::InvalidRange)
}

fn datetime_nanos_to_date(nanos: i64) -> Result<chrono::NaiveDate, DateRangeError> {
    let (date, _) = split_datetime_nanos(nanos)?;
    Ok(date)
}

fn split_datetime_nanos(nanos: i64) -> Result<(chrono::NaiveDate, i64), DateRangeError> {
    let days = nanos.div_euclid(Timedelta::NANOS_PER_DAY);
    let time_nanos = nanos.rem_euclid(Timedelta::NANOS_PER_DAY);
    let epoch = chrono::NaiveDate::from_ymd_opt(1970, 1, 1).ok_or(DateRangeError::InvalidRange)?;
    let date = epoch
        .checked_add_signed(chrono::Duration::days(days))
        .ok_or(DateRangeError::InvalidRange)?;
    Ok((date, time_nanos))
}

fn date_to_midnight_nanos(date: chrono::NaiveDate) -> Result<i64, DateRangeError> {
    let dt = date
        .and_hms_opt(0, 0, 0)
        .ok_or(DateRangeError::InvalidRange)?;
    dt.and_utc()
        .timestamp_nanos_opt()
        .ok_or(DateRangeError::InvalidRange)
}

fn date_and_time_to_nanos(date: chrono::NaiveDate, time_nanos: i64) -> Result<i64, DateRangeError> {
    date_to_midnight_nanos(date)?
        .checked_add(time_nanos)
        .ok_or(DateRangeError::InvalidRange)
}

fn checked_day_step(
    date: chrono::NaiveDate,
    days: i64,
) -> Result<chrono::NaiveDate, DateRangeError> {
    date.checked_add_signed(chrono::Duration::days(days))
        .ok_or(DateRangeError::InvalidRange)
}

fn is_business_day(date: chrono::NaiveDate) -> bool {
    use chrono::{Datelike, Weekday};

    !matches!(date.weekday(), Weekday::Sat | Weekday::Sun)
}

fn next_business_day(mut date: chrono::NaiveDate) -> Result<chrono::NaiveDate, DateRangeError> {
    while !is_business_day(date) {
        date = checked_day_step(date, 1)?;
    }
    Ok(date)
}

fn previous_business_day(mut date: chrono::NaiveDate) -> Result<chrono::NaiveDate, DateRangeError> {
    while !is_business_day(date) {
        date = checked_day_step(date, -1)?;
    }
    Ok(date)
}

fn collect_business_days_from_start(
    start: chrono::NaiveDate,
    periods: usize,
) -> Result<Vec<i64>, DateRangeError> {
    let mut values = Vec::with_capacity(periods);
    let mut date = next_business_day(start)?;
    while values.len() < periods {
        values.push(date_to_midnight_nanos(date)?);
        date = next_business_day(checked_day_step(date, 1)?)?;
    }
    Ok(values)
}

fn collect_business_days_through_end(
    end: chrono::NaiveDate,
    periods: usize,
) -> Result<Vec<i64>, DateRangeError> {
    let mut values = Vec::with_capacity(periods);
    let mut date = previous_business_day(end)?;
    while values.len() < periods {
        values.push(date_to_midnight_nanos(date)?);
        date = previous_business_day(checked_day_step(date, -1)?)?;
    }
    values.reverse();
    Ok(values)
}

fn collect_business_days_between(
    start: chrono::NaiveDate,
    end: chrono::NaiveDate,
) -> Result<Vec<i64>, DateRangeError> {
    if end < start {
        return Err(DateRangeError::InvalidRange);
    }

    let mut values = Vec::new();
    let mut date = next_business_day(start)?;
    while date <= end {
        values.push(date_to_midnight_nanos(date)?);
        date = next_business_day(checked_day_step(date, 1)?)?;
    }
    Ok(values)
}

/// A small subset of pandas `pandas.tseries.offsets` date offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateOffset {
    Day(i32),
    BusinessDay(i32),
    MonthEnd(i32),
}

/// Apply a date offset to a timestamp string and return nanoseconds since epoch.
///
/// This mirrors `pd.Timestamp(timestamp) + pd.offsets.<Offset>(n)` for the
/// supported offsets.
pub fn apply_date_offset(timestamp: &str, offset: DateOffset) -> Result<i64, DateRangeError> {
    let nanos = parse_datetime_to_nanos(timestamp)?;
    apply_date_offset_to_nanos(nanos, offset)
}

/// Apply a date offset to a nanosecond timestamp.
pub fn apply_date_offset_to_nanos(nanos: i64, offset: DateOffset) -> Result<i64, DateRangeError> {
    match offset {
        DateOffset::Day(days) => nanos
            .checked_add(
                i64::from(days)
                    .checked_mul(Timedelta::NANOS_PER_DAY)
                    .ok_or(DateRangeError::InvalidRange)?,
            )
            .ok_or(DateRangeError::InvalidRange),
        DateOffset::BusinessDay(days) => {
            let (date, time_nanos) = split_datetime_nanos(nanos)?;
            let shifted = apply_business_day_offset(date, days)?;
            date_and_time_to_nanos(shifted, time_nanos)
        }
        DateOffset::MonthEnd(months) => {
            let (date, time_nanos) = split_datetime_nanos(nanos)?;
            let shifted = apply_month_end_offset(date, months)?;
            date_and_time_to_nanos(shifted, time_nanos)
        }
    }
}

fn apply_business_day_offset(
    date: chrono::NaiveDate,
    days: i32,
) -> Result<chrono::NaiveDate, DateRangeError> {
    if days == 0 {
        return next_business_day(date);
    }

    let mut shifted = date;
    if days > 0 {
        for _ in 0..days.unsigned_abs() {
            shifted = next_business_day(checked_day_step(shifted, 1)?)?;
        }
    } else {
        for _ in 0..days.unsigned_abs() {
            shifted = previous_business_day(checked_day_step(shifted, -1)?)?;
        }
    }
    Ok(shifted)
}

fn last_day_of_month(year: i32, month: u32) -> Result<chrono::NaiveDate, DateRangeError> {
    let (next_year, next_month) = if month == 12 {
        (year.checked_add(1).ok_or(DateRangeError::InvalidRange)?, 1)
    } else {
        (year, month + 1)
    };
    let first_next_month = chrono::NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .ok_or(DateRangeError::InvalidRange)?;
    checked_day_step(first_next_month, -1)
}

fn add_months_to_month_end(
    date: chrono::NaiveDate,
    months: i32,
) -> Result<chrono::NaiveDate, DateRangeError> {
    use chrono::Datelike;

    let month_index = i64::from(date.year())
        .checked_mul(12)
        .and_then(|value| value.checked_add(i64::from(date.month()) - 1))
        .and_then(|value| value.checked_add(i64::from(months)))
        .ok_or(DateRangeError::InvalidRange)?;
    let year =
        i32::try_from(month_index.div_euclid(12)).map_err(|_| DateRangeError::InvalidRange)?;
    let month =
        u32::try_from(month_index.rem_euclid(12) + 1).map_err(|_| DateRangeError::InvalidRange)?;
    last_day_of_month(year, month)
}

fn month_ordinal(date: chrono::NaiveDate) -> i64 {
    use chrono::Datelike;

    i64::from(date.year()) * 12 + i64::from(date.month()) - 1
}

fn apply_month_end_offset(
    date: chrono::NaiveDate,
    months: i32,
) -> Result<chrono::NaiveDate, DateRangeError> {
    use chrono::Datelike;

    let current_month_end = last_day_of_month(date.year(), date.month())?;
    if months == 0 {
        return if date == current_month_end {
            Ok(date)
        } else {
            Ok(current_month_end)
        };
    }

    let month_steps = if months > 0 && date != current_month_end {
        months - 1
    } else {
        months
    };
    add_months_to_month_end(current_month_end, month_steps)
}

fn fixed_frequency_name(diff: i64) -> Option<String> {
    if diff <= 0 {
        return None;
    }

    let units = [
        (Timedelta::NANOS_PER_DAY, "D"),
        (Timedelta::NANOS_PER_HOUR, "h"),
        (Timedelta::NANOS_PER_MIN, "min"),
        (Timedelta::NANOS_PER_SEC, "s"),
        (Timedelta::NANOS_PER_MILLI, "ms"),
        (Timedelta::NANOS_PER_MICRO, "us"),
        (1, "ns"),
    ];
    for (unit_nanos, suffix) in units {
        if diff % unit_nanos == 0 {
            let count = diff / unit_nanos;
            return if count == 1 {
                Some(suffix.to_owned())
            } else {
                Some(format!("{count}{suffix}"))
            };
        }
    }
    None
}

fn infer_business_day_freq(dates: &[(chrono::NaiveDate, i64)]) -> Option<String> {
    if dates.iter().any(|(date, _)| !is_business_day(*date)) {
        return None;
    }
    let first_time = dates[0].1;
    if dates.iter().any(|(_, time)| *time != first_time) {
        return None;
    }
    for window in dates.windows(2) {
        let expected = next_business_day(checked_day_step(window[0].0, 1).ok()?).ok()?;
        if window[1].0 != expected {
            return None;
        }
    }
    Some("B".to_owned())
}

fn infer_month_end_freq(dates: &[(chrono::NaiveDate, i64)]) -> Option<String> {
    use chrono::Datelike;

    let first_time = dates[0].1;
    if dates.iter().any(|(_, time)| *time != first_time) {
        return None;
    }
    for (date, _) in dates {
        if *date != last_day_of_month(date.year(), date.month()).ok()? {
            return None;
        }
    }

    let step = month_ordinal(dates[1].0) - month_ordinal(dates[0].0);
    if step <= 0 {
        return None;
    }
    if dates
        .windows(2)
        .all(|window| month_ordinal(window[1].0) - month_ordinal(window[0].0) == step)
    {
        if step == 1 {
            Some("ME".to_owned())
        } else {
            Some(format!("{step}ME"))
        }
    } else {
        None
    }
}

/// Infer a pandas-style frequency string from a DatetimeIndex.
///
/// Returns `Ok(None)` for irregular or duplicate timestamp sequences. Returns
/// an error for the pandas-compatible "fewer than 3 dates" case.
pub fn infer_freq(index: &Index) -> Result<Option<String>, DateRangeError> {
    let mut values = Vec::with_capacity(index.len());
    for label in index.labels() {
        match label {
            IndexLabel::Datetime64(value) if *value != i64::MIN => values.push(*value),
            IndexLabel::Datetime64(_) => return Ok(None),
            _ => {
                return Err(DateRangeError::ParseError(
                    "expected datetime64 index".to_owned(),
                ));
            }
        }
    }
    infer_freq_from_nanos(&values)
}

/// Infer a pandas-style frequency string from timestamp strings.
pub fn infer_freq_from_timestamps(timestamps: &[&str]) -> Result<Option<String>, DateRangeError> {
    let values: Vec<i64> = timestamps
        .iter()
        .map(|timestamp| parse_datetime_to_nanos(timestamp))
        .collect::<Result<_, _>>()?;
    infer_freq_from_nanos(&values)
}

/// Infer a pandas-style frequency string from nanosecond timestamps.
pub fn infer_freq_from_nanos(values: &[i64]) -> Result<Option<String>, DateRangeError> {
    if values.len() < 3 {
        return Err(DateRangeError::InsufficientDates);
    }
    if values.windows(2).any(|window| window[1] <= window[0]) {
        return Ok(None);
    }

    let first_diff = values[1] - values[0];
    if values
        .windows(2)
        .all(|window| window[1] - window[0] == first_diff)
    {
        return Ok(fixed_frequency_name(first_diff));
    }

    let dates: Vec<(chrono::NaiveDate, i64)> = values
        .iter()
        .map(|value| split_datetime_nanos(*value))
        .collect::<Result<_, _>>()?;
    if let Some(freq) = infer_business_day_freq(&dates) {
        return Ok(Some(freq));
    }
    if let Some(freq) = infer_month_end_freq(&dates) {
        return Ok(Some(freq));
    }

    Ok(None)
}

/// Create a DatetimeIndex with evenly spaced values.
///
/// Analogous to `pd.date_range()`. Must specify exactly two of:
/// start, end, periods. Frequency defaults to 1 day.
///
/// # Examples
/// ```
/// use fp_index::date_range;
/// use fp_types::Timedelta;
///
/// let idx = date_range(
///     Some("2024-01-01"),
///     None,
///     Some(3),
///     Timedelta::NANOS_PER_DAY,
///     None,
/// ).unwrap();
/// assert_eq!(idx.len(), 3);
/// ```
pub fn date_range(
    start: Option<&str>,
    end: Option<&str>,
    periods: Option<usize>,
    freq: i64,
    name: Option<&str>,
) -> Result<Index, DateRangeError> {
    if freq <= 0 {
        return Err(DateRangeError::NonPositiveFreq);
    }

    let start_ns = start.map(parse_datetime_to_nanos).transpose()?;
    let end_ns = end.map(parse_datetime_to_nanos).transpose()?;

    let (start_val, count) = match (start_ns, end_ns, periods) {
        (Some(s), Some(e), None) => {
            if e < s {
                return Err(DateRangeError::InvalidRange);
            }
            let span = e.checked_sub(s).ok_or(DateRangeError::InvalidRange)?;
            let n = (span / freq + 1) as usize;
            (s, n)
        }
        (Some(s), None, Some(p)) => (s, p),
        (None, Some(e), Some(p)) => {
            let offset = checked_date_range_offset(p.saturating_sub(1), freq)?;
            let s = e.checked_sub(offset).ok_or(DateRangeError::InvalidRange)?;
            (s, p)
        }
        (Some(_), Some(_), Some(_)) => return Err(DateRangeError::TooManyParams),
        _ => return Err(DateRangeError::InsufficientParams),
    };

    let last_offset = checked_date_range_offset(count.saturating_sub(1), freq)?;
    start_val
        .checked_add(last_offset)
        .ok_or(DateRangeError::InvalidRange)?;

    let nanos: Vec<i64> = (0..count)
        .map(|i| {
            let offset = checked_date_range_offset(i, freq)?;
            start_val
                .checked_add(offset)
                .ok_or(DateRangeError::InvalidRange)
        })
        .collect::<Result<_, _>>()?;
    let mut idx = Index::from_datetime64(nanos);
    if let Some(n) = name {
        idx = idx.set_name(n);
    }
    Ok(idx)
}

fn checked_date_range_offset(steps: usize, freq: i64) -> Result<i64, DateRangeError> {
    let steps = i64::try_from(steps).map_err(|_| DateRangeError::InvalidRange)?;
    steps.checked_mul(freq).ok_or(DateRangeError::InvalidRange)
}

/// Create a DatetimeIndex with default weekday-only business-day values.
///
/// Analogous to `pd.bdate_range(..., freq="B")` for the default Monday-Friday
/// calendar. Exactly two of start, end, and periods must be specified.
pub fn bdate_range(
    start: Option<&str>,
    end: Option<&str>,
    periods: Option<usize>,
    name: Option<&str>,
) -> Result<Index, DateRangeError> {
    let start_date = start
        .map(parse_datetime_to_nanos)
        .transpose()?
        .map(datetime_nanos_to_date)
        .transpose()?;
    let end_date = end
        .map(parse_datetime_to_nanos)
        .transpose()?
        .map(datetime_nanos_to_date)
        .transpose()?;

    let nanos = match (start_date, end_date, periods) {
        (Some(start), Some(end), None) => collect_business_days_between(start, end)?,
        (Some(start), None, Some(periods)) => collect_business_days_from_start(start, periods)?,
        (None, Some(end), Some(periods)) => collect_business_days_through_end(end, periods)?,
        (Some(_), Some(_), Some(_)) => return Err(DateRangeError::TooManyParams),
        _ => return Err(DateRangeError::InsufficientParams),
    };

    let mut idx = Index::from_datetime64(nanos);
    if let Some(n) = name {
        idx = idx.set_name(n);
    }
    Ok(idx)
}

// ── MultiIndex ──────────────────────────────────────────────────────────

/// A hierarchical (multi-level) index for DataFrames and Series.
///
/// Stores multiple levels of labels as separate vectors (columnar layout),
/// analogous to pandas `pd.MultiIndex`. Each row position has one label
/// per level. The combination of labels across all levels forms the
/// composite key for that row.
///
/// This type exists alongside `Index` and can be converted to/from it.
/// Full DataFrame integration is a future step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultiIndex {
    /// One `Vec<IndexLabel>` per level, all the same length (= nrows).
    levels: Vec<Vec<IndexLabel>>,
    /// Optional name for each level.
    names: Vec<Option<String>>,
}

impl MultiIndex {
    /// Number of levels in this MultiIndex.
    #[must_use]
    pub fn nlevels(&self) -> usize {
        self.levels.len()
    }

    /// Number of rows (entries) in this MultiIndex.
    #[must_use]
    pub fn len(&self) -> usize {
        self.levels.first().map_or(0, Vec::len)
    }

    /// Whether this MultiIndex has zero entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Compare two rows lexicographically across all levels.
    ///
    /// Private helper for sortedness predicates. Returns `Ordering::Equal`
    /// only when every level value matches exactly.
    fn row_cmp(&self, a: usize, b: usize) -> std::cmp::Ordering {
        for level in 0..self.nlevels() {
            let ord = self.levels[level][a].cmp(&self.levels[level][b]);
            if ord != std::cmp::Ordering::Equal {
                return ord;
            }
        }
        std::cmp::Ordering::Equal
    }

    /// Whether this MultiIndex is sorted in lexicographic (row-major) order.
    ///
    /// Matches `pd.MultiIndex.is_monotonic_increasing`. Row `i` must be less
    /// than or equal to row `i+1` under level-by-level comparison. Empty or
    /// single-row indexes return `true` (trivially sorted).
    ///
    /// Per br-frankenpandas-w4uu: pandas `df.loc['A':'B']` on a MultiIndex
    /// raises `KeyError: MultiIndex slicing requires the index to be
    /// lexsorted` when this predicate is false. fp-frame's range-slice
    /// callers should gate on this before delegating to `slice_locs`.
    #[must_use]
    pub fn is_monotonic_increasing(&self) -> bool {
        if self.len() <= 1 {
            return true;
        }
        (0..self.len() - 1).all(|i| self.row_cmp(i, i + 1) != std::cmp::Ordering::Greater)
    }

    /// Whether this MultiIndex is sorted in strictly descending order.
    ///
    /// Matches `pd.MultiIndex.is_monotonic_decreasing`. Row `i` must be
    /// greater than or equal to row `i+1`. Empty / single-row: `true`.
    #[must_use]
    pub fn is_monotonic_decreasing(&self) -> bool {
        if self.len() <= 1 {
            return true;
        }
        (0..self.len() - 1).all(|i| self.row_cmp(i, i + 1) != std::cmp::Ordering::Less)
    }

    /// Alias for `is_monotonic_increasing` matching `pd.MultiIndex.is_lexsorted`
    /// semantics (pandas deprecated the `is_lexsorted` name in 1.x; we keep
    /// it as a convenience alias for migrated code).
    #[must_use]
    pub fn is_lexsorted(&self) -> bool {
        self.is_monotonic_increasing()
    }

    /// Level names.
    #[must_use]
    pub fn names(&self) -> &[Option<String>] {
        &self.names
    }

    /// Scalar index name, matching `pd.MultiIndex.name`.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        None
    }

    /// Number of entries, matching `pd.MultiIndex.size`.
    #[must_use]
    pub fn size(&self) -> usize {
        self.len()
    }

    /// Shape of this one-dimensional index, matching `pd.MultiIndex.shape`.
    #[must_use]
    pub fn shape(&self) -> (usize,) {
        (self.len(),)
    }

    /// Number of dimensions, matching `pd.MultiIndex.ndim`.
    #[must_use]
    pub fn ndim(&self) -> usize {
        1
    }

    /// Alias for `is_empty`, matching the pandas `.empty` property.
    #[must_use]
    pub fn empty(&self) -> bool {
        self.is_empty()
    }

    fn shift_unsupported_error() -> IndexError {
        IndexError::InvalidArgument(
            "This method is only implemented for DatetimeIndex, PeriodIndex and TimedeltaIndex; Got type MultiIndex"
                .to_owned(),
        )
    }

    /// Unsupported temporal shift, matching `pd.MultiIndex.shift(...)`.
    pub fn shift(&self, _periods: i64, _freq: Option<&str>) -> Result<Self, IndexError> {
        Err(Self::shift_unsupported_error())
    }

    fn astype_categorical_error() -> IndexError {
        IndexError::InvalidArgument(
            "> 1 ndim Categorical are not supported at this time".to_owned(),
        )
    }

    fn astype_unsupported_dtype_error(dtype: &str) -> IndexError {
        IndexError::InvalidArgument(format!(
            "Setting a MultiIndex dtype to anything other than object is not supported; got {dtype}"
        ))
    }

    /// Cast labels to a different dtype, matching `pd.MultiIndex.astype(...)`.
    ///
    /// Pandas only supports the object dtype on MultiIndex; categorical raises
    /// `NotImplementedError` and any other dtype raises `TypeError`. Object
    /// returns a clone of the index.
    pub fn astype(&self, dtype: &str) -> Result<Self, IndexError> {
        match dtype {
            "object" | "O" => Ok(self.clone()),
            "category" => Err(Self::astype_categorical_error()),
            other => Err(Self::astype_unsupported_dtype_error(other)),
        }
    }

    fn diff_unsupported_error() -> IndexError {
        IndexError::InvalidArgument(
            "cannot perform __sub__ with this index type: MultiIndex".to_owned(),
        )
    }

    /// Unsupported positional differencing, matching `pd.MultiIndex.diff(...)`.
    ///
    /// Pandas defines `Index.diff` as `self - self.shift(periods)` and raises
    /// `TypeError` because tuple-valued levels do not support subtraction.
    pub fn diff(&self, _periods: i64) -> Result<Self, IndexError> {
        Err(Self::diff_unsupported_error())
    }

    fn round_unsupported_error() -> IndexError {
        IndexError::InvalidArgument(
            "loop of ufunc does not support argument 0 of type tuple which has no callable rint method"
                .to_owned(),
        )
    }

    /// Unsupported numeric rounding, matching `pd.MultiIndex.round(...)`.
    ///
    /// Pandas applies `np.around` to the underlying values; tuple-valued
    /// MultiIndex labels do not support `rint`, so this surface always rejects.
    pub fn round(&self, _decimals: i32) -> Result<Self, IndexError> {
        Err(Self::round_unsupported_error())
    }

    fn string_accessor_error() -> IndexError {
        IndexError::InvalidArgument(
            "Can only use .str accessor with Index, not MultiIndex".to_owned(),
        )
    }

    /// Unsupported string accessor, matching `pd.MultiIndex.str`.
    pub fn r#str(&self) -> Result<(), IndexError> {
        Err(Self::string_accessor_error())
    }

    fn asof_comparison_type_name(&self) -> &'static str {
        match self.levels.first().and_then(|level| level.first()) {
            Some(IndexLabel::Int64(_)) => "int",
            Some(IndexLabel::Float64(_)) => "float",
            Some(IndexLabel::Bool(_)) => "bool",
            Some(IndexLabel::Utf8(_)) => "str",
            Some(IndexLabel::Timedelta64(_)) => "Timedelta",
            Some(IndexLabel::Datetime64(_)) => "Timestamp",
            Some(IndexLabel::Null(fp_types::NullKind::Null)) => "NoneType",
            Some(IndexLabel::Null(fp_types::NullKind::NaN)) => "float",
            Some(IndexLabel::Null(fp_types::NullKind::NaT)) => "NaTType",
            None => "object",
        }
    }

    fn asof_unsupported_error(&self) -> IndexError {
        IndexError::InvalidArgument(format!(
            "'<' not supported between instances of 'tuple' and '{}'",
            self.asof_comparison_type_name()
        ))
    }

    /// Unsupported nearest-key lookup, matching `pd.MultiIndex.asof(...)`.
    pub fn asof(&self, _key: &[IndexLabel]) -> Result<Option<Vec<IndexLabel>>, IndexError> {
        if self.is_empty() {
            return Ok(None);
        }
        Err(self.asof_unsupported_error())
    }

    fn asof_locs_no_mask_error() -> IndexError {
        IndexError::InvalidArgument("object too deep for desired array".to_owned())
    }

    fn asof_locs_empty_mask_error() -> IndexError {
        IndexError::InvalidArgument("attempt to get argmax of an empty sequence".to_owned())
    }

    fn asof_locs_empty_take_error() -> IndexError {
        IndexError::InvalidArgument("cannot do a non-empty take from an empty axes.".to_owned())
    }

    fn asof_locs_mask_length_error(expected: usize, actual: usize) -> IndexError {
        IndexError::InvalidArgument(format!(
            "boolean index did not match indexed array along axis 0; size of axis is {expected} but size of corresponding boolean axis is {actual}"
        ))
    }

    fn asof_locs_broadcast_error(where_len: usize) -> IndexError {
        IndexError::InvalidArgument(format!(
            "operands could not be broadcast together with shapes ({where_len},) (2,)"
        ))
    }

    /// Unsupported nearest-position lookup, matching `pd.MultiIndex.asof_locs(...)`.
    pub fn asof_locs(
        &self,
        where_index: &Self,
        mask: Option<&[bool]>,
    ) -> Result<Vec<Option<usize>>, IndexError> {
        let Some(mask) = mask else {
            return Err(Self::asof_locs_no_mask_error());
        };
        if mask.len() != self.len() {
            return Err(Self::asof_locs_mask_length_error(self.len(), mask.len()));
        }
        if mask.is_empty() && self.is_empty() && where_index.is_empty() {
            return Err(Self::asof_locs_empty_mask_error());
        }
        if mask.iter().all(|include| !*include) && !where_index.is_empty() {
            return Err(Self::asof_locs_empty_take_error());
        }
        Err(Self::asof_locs_broadcast_error(where_index.len()))
    }

    /// Set the names for all levels.
    #[must_use]
    pub fn set_names(mut self, names: Vec<Option<String>>) -> Self {
        // Pad or truncate to match nlevels.
        self.names = names;
        self.names.resize(self.nlevels(), None);
        self
    }

    /// Rename all MultiIndex levels, matching `pd.MultiIndex.rename(names)`.
    ///
    /// Unlike [`Self::set_names`], pandas rename requires one name per level
    /// and returns a renamed clone without mutating the source index.
    pub fn rename(&self, names: Vec<Option<String>>) -> Result<Self, IndexError> {
        if names.len() != self.nlevels() {
            return Err(IndexError::LengthMismatch {
                expected: self.nlevels(),
                actual: names.len(),
                context: "MultiIndex.rename names length".to_owned(),
            });
        }
        Ok(Self {
            levels: self.levels.clone(),
            names,
        })
    }

    /// Rename one MultiIndex level, matching `pd.MultiIndex.rename(name, level=...)`.
    pub fn rename_level(&self, name: Option<String>, level: usize) -> Result<Self, IndexError> {
        if level >= self.nlevels() {
            return Err(IndexError::OutOfBounds {
                position: level,
                length: self.nlevels(),
            });
        }
        let mut names = self.names.clone();
        names[level] = name;
        Ok(Self {
            levels: self.levels.clone(),
            names,
        })
    }

    fn shared_names(&self, other: &Self) -> Vec<Option<String>> {
        self.names
            .iter()
            .zip(&other.names)
            .map(
                |(left, right)| {
                    if left == right { left.clone() } else { None }
                },
            )
            .collect()
    }

    fn ensure_same_nlevels(&self, other: &Self) -> Result<(), IndexError> {
        if self.nlevels() != other.nlevels() {
            return Err(IndexError::LengthMismatch {
                expected: self.nlevels(),
                actual: other.nlevels(),
                context: "MultiIndex level count mismatch".to_owned(),
            });
        }
        Ok(())
    }

    fn tuple_at(&self, row: usize) -> Vec<IndexLabel> {
        self.levels.iter().map(|level| level[row].clone()).collect()
    }

    fn take_existing_positions(&self, positions: &[usize]) -> Self {
        let levels = self
            .levels
            .iter()
            .map(|level| {
                positions
                    .iter()
                    .map(|&position| level[position].clone())
                    .collect()
            })
            .collect();
        Self {
            levels,
            names: self.names.clone(),
        }
    }

    fn missing_label_for_level(&self, level_idx: usize) -> IndexLabel {
        self.levels[level_idx]
            .iter()
            .find(|label| label.is_missing())
            .cloned()
            .unwrap_or(IndexLabel::Datetime64(i64::MIN))
    }

    fn from_tuples_with_names(
        tuples: Vec<Vec<IndexLabel>>,
        names: Vec<Option<String>>,
    ) -> Result<Self, IndexError> {
        Ok(Self::from_tuples(tuples)?.set_names(names))
    }

    /// Unique labels for each level, preserving first-seen order.
    ///
    /// Matches `pd.MultiIndex.levels`. Missing labels are excluded from the
    /// level catalog and receive `-1` in `codes()`.
    #[must_use]
    pub fn levels(&self) -> Vec<Index> {
        self.levels
            .iter()
            .enumerate()
            .map(|(level_idx, level)| {
                let mut seen = FxHashMap::<&IndexLabel, ()>::default();
                let labels = level
                    .iter()
                    .filter(|label| !label.is_missing() && seen.insert(label, ()).is_none())
                    .cloned()
                    .collect();
                let mut index = Index::new(labels);
                if let Some(name) = self.names.get(level_idx).and_then(|name| name.as_ref()) {
                    index = index.set_name(name);
                }
                index
            })
            .collect()
    }

    /// Integer level codes for each row, matching `pd.MultiIndex.codes`.
    ///
    /// Missing labels receive code `-1`; all other labels are encoded by their
    /// first-seen position in the corresponding `levels()` entry.
    #[must_use]
    pub fn codes(&self) -> Vec<Vec<isize>> {
        self.levels
            .iter()
            .map(|level| {
                let mut positions = FxHashMap::<IndexLabel, isize>::default();
                let mut next_code = 0_isize;
                level
                    .iter()
                    .map(|label| {
                        if label.is_missing() {
                            -1
                        } else if let Some(code) = positions.get(label) {
                            *code
                        } else {
                            let code = next_code;
                            positions.insert(label.clone(), code);
                            next_code += 1;
                            code
                        }
                    })
                    .collect()
            })
            .collect()
    }

    /// Cardinality of each level, matching `pd.MultiIndex.levshape`.
    #[must_use]
    pub fn levshape(&self) -> Vec<usize> {
        self.levels().iter().map(Index::len).collect()
    }

    /// Materialize every composite key as an owned tuple.
    ///
    /// Matches `pd.MultiIndex.to_list()` / `tolist()`.
    #[must_use]
    pub fn to_list(&self) -> Vec<Vec<IndexLabel>> {
        (0..self.len()).map(|row| self.tuple_at(row)).collect()
    }

    /// Alias for `to_list`, matching `pd.MultiIndex.tolist()`.
    #[must_use]
    pub fn tolist(&self) -> Vec<Vec<IndexLabel>> {
        self.to_list()
    }

    /// Object-array-shaped materialization, matching `pd.MultiIndex.to_numpy`.
    #[must_use]
    pub fn to_numpy(&self) -> Vec<Vec<IndexLabel>> {
        self.to_list()
    }

    /// Alias for `to_numpy`, matching `pd.MultiIndex.values`.
    #[must_use]
    pub fn values(&self) -> Vec<Vec<IndexLabel>> {
        self.to_numpy()
    }

    /// Alias for `to_numpy`, matching `pd.MultiIndex.array`.
    #[must_use]
    pub fn array(&self) -> Vec<Vec<IndexLabel>> {
        self.to_numpy()
    }

    /// Alias for `to_numpy`, matching `pd.MultiIndex.ravel()`.
    #[must_use]
    pub fn ravel(&self) -> Vec<Vec<IndexLabel>> {
        self.to_numpy()
    }

    /// Return a shallow clone view, matching `pd.MultiIndex.view`.
    #[must_use]
    pub fn view(&self) -> Self {
        self.clone()
    }

    /// MultiIndex transpose is identity, matching `pd.MultiIndex.transpose`.
    #[must_use]
    pub fn transpose(&self) -> Self {
        self.clone()
    }

    /// Alias for `transpose`, matching `pd.MultiIndex.T`.
    #[allow(non_snake_case)]
    #[must_use]
    pub fn T(&self) -> Self {
        self.transpose()
    }

    /// Row materialization for `pd.MultiIndex.to_frame(index=False)`.
    ///
    /// `fp-frame` owns the richer DataFrame return type; the index crate keeps
    /// the level-by-level row payload that callers can lift into a frame.
    #[must_use]
    pub fn to_frame(&self) -> Vec<Vec<IndexLabel>> {
        self.to_list()
    }

    /// Series-shaped materialization using tuple keys as both index and value.
    ///
    /// This mirrors `pd.MultiIndex.to_series()` at the payload level while
    /// avoiding a dependency from `fp-index` back into `fp-frame`.
    #[must_use]
    pub fn to_series(&self) -> Vec<(Vec<IndexLabel>, Vec<IndexLabel>)> {
        self.to_list()
            .into_iter()
            .map(|tuple| (tuple.clone(), tuple))
            .collect()
    }

    /// Stringify each tuple in row order, matching `pd.MultiIndex.format()`.
    #[must_use]
    pub fn format(&self) -> Vec<String> {
        self.to_list()
            .into_iter()
            .map(|tuple| {
                let parts: Vec<String> = tuple.into_iter().map(|label| label.to_string()).collect();
                format!("({})", parts.join(", "))
            })
            .collect()
    }

    /// Approximate memory footprint of all level labels and codes.
    ///
    /// `deep=false` counts fixed-width labels and `String` headers; `deep=true`
    /// additionally counts string bytes, mirroring `Index::memory_usage`.
    #[must_use]
    pub fn memory_usage(&self, deep: bool) -> usize {
        self.levels
            .iter()
            .flatten()
            .map(|label| match label {
                IndexLabel::Int64(_)
                | IndexLabel::Float64(_)
                | IndexLabel::Timedelta64(_)
                | IndexLabel::Datetime64(_)
                | IndexLabel::Null(_) => 8,
                IndexLabel::Bool(_) => 1,
                IndexLabel::Utf8(value) => {
                    if deep {
                        std::mem::size_of::<String>() + value.len()
                    } else {
                        std::mem::size_of::<String>()
                    }
                }
            })
            .sum::<usize>()
            + self.nlevels() * self.len() * std::mem::size_of::<isize>()
    }

    /// Shallow memory footprint, matching `pd.MultiIndex.nbytes`.
    #[must_use]
    pub fn nbytes(&self) -> usize {
        self.memory_usage(false)
    }

    /// Pandas dtype string for MultiIndex itself.
    #[must_use]
    pub fn dtype(&self) -> &'static str {
        "object"
    }

    /// Dtype string for each level, matching `pd.MultiIndex.dtypes`.
    #[must_use]
    pub fn dtypes(&self) -> Vec<&'static str> {
        self.levels
            .iter()
            .map(|level| Index::new(level.clone()).dtype())
            .collect()
    }

    /// Pandas-style inferred type for MultiIndex values.
    #[must_use]
    pub fn inferred_type(&self) -> &'static str {
        "mixed"
    }

    /// Infer object labels without changing this typed representation.
    #[must_use]
    pub fn infer_objects(&self) -> Self {
        self.clone()
    }

    /// Whether this MultiIndex can hold integer labels as scalar keys.
    #[must_use]
    pub fn holds_integer(&self) -> bool {
        false
    }

    /// Return the sole tuple, matching `pd.MultiIndex.item()`.
    pub fn item(&self) -> Result<Vec<IndexLabel>, IndexError> {
        if self.len() == 1 {
            Ok(self.tuple_at(0))
        } else {
            Err(IndexError::InvalidArgument(format!(
                "item requires exactly one tuple, got {}",
                self.len()
            )))
        }
    }

    /// Return a shallow copy, matching `pd.MultiIndex.copy()`.
    #[must_use]
    pub fn copy(&self) -> Self {
        self.clone()
    }

    fn multi_index_isna_error() -> IndexError {
        IndexError::InvalidArgument("isna is not defined for MultiIndex".to_owned())
    }

    /// Unsupported missing-label check, matching `pd.MultiIndex.hasnans`.
    pub fn hasnans(&self) -> Result<bool, IndexError> {
        Err(Self::multi_index_isna_error())
    }

    /// Unsupported missing-label mask, matching `pd.MultiIndex.isna()`.
    pub fn isna(&self) -> Result<Vec<bool>, IndexError> {
        Err(Self::multi_index_isna_error())
    }

    /// Alias for `isna`, matching `pd.MultiIndex.isnull`.
    pub fn isnull(&self) -> Result<Vec<bool>, IndexError> {
        Err(Self::multi_index_isna_error())
    }

    /// Unsupported inverse missing-label mask, matching `pd.MultiIndex.notna()`.
    pub fn notna(&self) -> Result<Vec<bool>, IndexError> {
        Err(Self::multi_index_isna_error())
    }

    /// Alias for `notna`, matching `pd.MultiIndex.notnull`.
    pub fn notnull(&self) -> Result<Vec<bool>, IndexError> {
        Err(Self::multi_index_isna_error())
    }

    /// Replace missing labels in every level with one scalar label.
    #[must_use]
    pub fn fillna(&self, value: &IndexLabel) -> Self {
        let levels = self
            .levels
            .iter()
            .map(|level| {
                level
                    .iter()
                    .map(|label| {
                        if label.is_missing() {
                            value.clone()
                        } else {
                            label.clone()
                        }
                    })
                    .collect()
            })
            .collect();
        Self {
            levels,
            names: self.names.clone(),
        }
    }

    /// Replace missing labels with one replacement per level.
    pub fn fillna_tuple(&self, values: &[IndexLabel]) -> Result<Self, IndexError> {
        if values.len() != self.nlevels() {
            return Err(IndexError::LengthMismatch {
                expected: self.nlevels(),
                actual: values.len(),
                context: "fillna_tuple replacement arity mismatch".to_owned(),
            });
        }
        let levels = self
            .levels
            .iter()
            .enumerate()
            .map(|(level_idx, level)| {
                level
                    .iter()
                    .map(|label| {
                        if label.is_missing() {
                            values[level_idx].clone()
                        } else {
                            label.clone()
                        }
                    })
                    .collect()
            })
            .collect();
        Ok(Self {
            levels,
            names: self.names.clone(),
        })
    }

    /// Replace tuples where `cond` is true with `value`.
    pub fn putmask(&self, cond: &[bool], value: Vec<IndexLabel>) -> Result<Self, IndexError> {
        if cond.len() != self.len() {
            return Err(IndexError::LengthMismatch {
                expected: self.len(),
                actual: cond.len(),
                context: "putmask condition length mismatch".to_owned(),
            });
        }
        if value.len() != self.nlevels() {
            return Err(IndexError::LengthMismatch {
                expected: self.nlevels(),
                actual: value.len(),
                context: "putmask tuple arity mismatch".to_owned(),
            });
        }
        let tuples = (0..self.len())
            .map(|row| {
                if cond[row] {
                    value.clone()
                } else {
                    self.tuple_at(row)
                }
            })
            .collect();
        Self::from_tuples_with_names(tuples, self.names.clone())
    }

    /// Keep original tuples where `cond` is true, otherwise use `other`.
    pub fn r#where(&self, cond: &[bool], other: Vec<IndexLabel>) -> Result<Self, IndexError> {
        if cond.len() != self.len() {
            return Err(IndexError::LengthMismatch {
                expected: self.len(),
                actual: cond.len(),
                context: "where condition length mismatch".to_owned(),
            });
        }
        if other.len() != self.nlevels() {
            return Err(IndexError::LengthMismatch {
                expected: self.nlevels(),
                actual: other.len(),
                context: "where tuple arity mismatch".to_owned(),
            });
        }
        let tuples = (0..self.len())
            .map(|row| {
                if cond[row] {
                    self.tuple_at(row)
                } else {
                    other.clone()
                }
            })
            .collect();
        Self::from_tuples_with_names(tuples, self.names.clone())
    }

    /// Map each composite tuple through a caller-supplied function.
    pub fn map<T, F>(&self, mut mapper: F) -> Vec<T>
    where
        F: FnMut(&[IndexLabel]) -> T,
    {
        (0..self.len())
            .map(|row| {
                let tuple = self.tuple_at(row);
                mapper(&tuple)
            })
            .collect()
    }

    /// Rebuild row labels using replacement level catalogs and current codes.
    pub fn set_levels(&self, new_levels: Vec<Vec<IndexLabel>>) -> Result<Self, IndexError> {
        if new_levels.len() != self.nlevels() {
            return Err(IndexError::LengthMismatch {
                expected: self.nlevels(),
                actual: new_levels.len(),
                context: "set_levels level count mismatch".to_owned(),
            });
        }
        let codes = self.codes();
        let mut levels = Vec::with_capacity(self.nlevels());
        for (level_idx, level_codes) in codes.into_iter().enumerate() {
            let mut level = Vec::with_capacity(self.len());
            for code in level_codes {
                if code == -1 {
                    level.push(self.missing_label_for_level(level_idx));
                    continue;
                }
                if code < -1 {
                    return Err(IndexError::InvalidArgument(format!(
                        "negative code {code} at level {level_idx}"
                    )));
                }
                let position = usize::try_from(code).map_err(|_| {
                    IndexError::InvalidArgument(format!("invalid code {code} at level {level_idx}"))
                })?;
                let label = new_levels[level_idx]
                    .get(position)
                    .ok_or(IndexError::OutOfBounds {
                        position,
                        length: new_levels[level_idx].len(),
                    })?;
                level.push(label.clone());
            }
            levels.push(level);
        }
        Ok(Self {
            levels,
            names: self.names.clone(),
        })
    }

    /// Rebuild row labels using replacement codes and current level catalogs.
    pub fn set_codes(&self, codes: Vec<Vec<isize>>) -> Result<Self, IndexError> {
        if codes.len() != self.nlevels() {
            return Err(IndexError::LengthMismatch {
                expected: self.nlevels(),
                actual: codes.len(),
                context: "set_codes level count mismatch".to_owned(),
            });
        }
        let catalogs = self.levels();
        let mut levels = Vec::with_capacity(self.nlevels());
        for (level_idx, level_codes) in codes.into_iter().enumerate() {
            if level_codes.len() != self.len() {
                return Err(IndexError::LengthMismatch {
                    expected: self.len(),
                    actual: level_codes.len(),
                    context: format!("set_codes level {level_idx} length mismatch"),
                });
            }
            let labels = catalogs[level_idx].labels();
            let mut level = Vec::with_capacity(self.len());
            for code in level_codes {
                if code == -1 {
                    level.push(self.missing_label_for_level(level_idx));
                    continue;
                }
                if code < -1 {
                    return Err(IndexError::InvalidArgument(format!(
                        "negative code {code} at level {level_idx}"
                    )));
                }
                let position = usize::try_from(code).map_err(|_| {
                    IndexError::InvalidArgument(format!("invalid code {code} at level {level_idx}"))
                })?;
                let label = labels.get(position).ok_or(IndexError::OutOfBounds {
                    position,
                    length: labels.len(),
                })?;
                level.push(label.clone());
            }
            levels.push(level);
        }
        Ok(Self {
            levels,
            names: self.names.clone(),
        })
    }

    /// Drop unused level labels. This representation stores row labels directly,
    /// so there is no separate unused catalog to prune.
    #[must_use]
    pub fn remove_unused_levels(&self) -> Self {
        self.clone()
    }

    /// Identity check, matching `pd.MultiIndex.is_`.
    #[must_use]
    pub fn is_(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }

    /// Whether this MultiIndex is boolean-typed.
    #[must_use]
    pub fn is_boolean(&self) -> bool {
        false
    }

    /// Whether this MultiIndex is categorical-typed.
    #[must_use]
    pub fn is_categorical(&self) -> bool {
        false
    }

    /// Whether this MultiIndex is floating-typed.
    #[must_use]
    pub fn is_floating(&self) -> bool {
        false
    }

    /// Whether this MultiIndex is integer-typed.
    #[must_use]
    pub fn is_integer(&self) -> bool {
        false
    }

    /// Whether this MultiIndex is interval-typed.
    #[must_use]
    pub fn is_interval(&self) -> bool {
        false
    }

    /// Whether this MultiIndex is numeric-typed.
    #[must_use]
    pub fn is_numeric(&self) -> bool {
        false
    }

    /// Whether this MultiIndex is object-backed.
    #[must_use]
    pub fn is_object(&self) -> bool {
        true
    }

    /// Compare row tuples only, matching `pd.MultiIndex.equals`.
    #[must_use]
    pub fn equals(&self, other: &Self) -> bool {
        self.levels == other.levels
    }

    /// Compare row tuples and level names, matching `pd.MultiIndex.identical`.
    #[must_use]
    pub fn identical(&self, other: &Self) -> bool {
        self.equals(other) && self.names == other.names
    }

    /// Compare unique level catalogs, matching `pd.MultiIndex.equal_levels`.
    #[must_use]
    pub fn equal_levels(&self, other: &Self) -> bool {
        self.levels() == other.levels()
    }

    /// Get the labels for a specific level.
    ///
    /// Matches `pd.MultiIndex.get_level_values(level)`.
    pub fn get_level_values(&self, level: usize) -> Result<Index, IndexError> {
        if level >= self.levels.len() {
            return Err(IndexError::OutOfBounds {
                position: level,
                length: self.levels.len(),
            });
        }
        let mut idx = Index::new(self.levels[level].clone());
        if let Some(name) = self.names.get(level).and_then(|n| n.as_ref()) {
            idx = idx.set_name(name);
        }
        Ok(idx)
    }

    /// Get the tuple of labels at a specific position.
    pub fn get_tuple(&self, position: usize) -> Option<Vec<&IndexLabel>> {
        if position >= self.len() {
            return None;
        }
        Some(self.levels.iter().map(|level| &level[position]).collect())
    }

    /// Select rows by positional index.
    pub fn take(&self, positions: &[usize]) -> Result<Self, IndexError> {
        for &position in positions {
            if position >= self.len() {
                return Err(IndexError::OutOfBounds {
                    position,
                    length: self.len(),
                });
            }
        }

        let mut levels = Vec::with_capacity(self.nlevels());
        for level in &self.levels {
            let selected = positions
                .iter()
                .map(|&position| level[position].clone())
                .collect();
            levels.push(selected);
        }

        Ok(Self {
            levels,
            names: self.names.clone(),
        })
    }

    /// Delete the tuple at a positional location.
    ///
    /// Matches `pd.MultiIndex.delete(loc)`.
    pub fn delete(&self, loc: usize) -> Result<Self, IndexError> {
        if loc >= self.len() {
            return Err(IndexError::OutOfBounds {
                position: loc,
                length: self.len(),
            });
        }
        let positions: Vec<usize> = (0..self.len()).filter(|&row| row != loc).collect();
        Ok(self.take_existing_positions(&positions))
    }

    /// Insert a tuple at a positional location.
    ///
    /// Matches `pd.MultiIndex.insert(loc, item)`. Inserting into an empty
    /// zero-level MultiIndex adopts the tuple arity as the new level count.
    pub fn insert(&self, loc: usize, item: Vec<IndexLabel>) -> Result<Self, IndexError> {
        if loc > self.len() {
            return Err(IndexError::OutOfBounds {
                position: loc,
                length: self.len(),
            });
        }
        if self.nlevels() == 0 {
            if loc != 0 {
                return Err(IndexError::OutOfBounds {
                    position: loc,
                    length: 0,
                });
            }
            return Self::from_tuples(vec![item]);
        }
        if item.len() != self.nlevels() {
            return Err(IndexError::LengthMismatch {
                expected: self.nlevels(),
                actual: item.len(),
                context: "insert tuple arity mismatch".to_owned(),
            });
        }

        let mut levels = self.levels.clone();
        for (level_idx, label) in item.into_iter().enumerate() {
            levels[level_idx].insert(loc, label);
        }
        Ok(Self {
            levels,
            names: self.names.clone(),
        })
    }

    /// Drop every occurrence of the provided tuples.
    ///
    /// Matches `pd.MultiIndex.drop(labels)` with the default fail-closed
    /// behavior for missing labels.
    pub fn drop(&self, labels_to_drop: &[Vec<IndexLabel>]) -> Result<Self, IndexError> {
        for label in labels_to_drop {
            self.validate_key_arity(label, false)?;
        }
        // Reuse one key buffer across rows instead of materializing every row
        // tuple via to_list() (which allocates a Vec per row). `found` stores
        // references into `labels_to_drop` (stable across iterations) — obtained
        // from drop_set.get() — so the reused buffer never escapes. FxHashSet
        // replaces the std SipHash set. Bit-identical: same retained positions
        // and same missing-key detection (value-based membership).
        let drop_set: FxHashSet<&Vec<IndexLabel>> = labels_to_drop.iter().collect();
        let mut found: FxHashSet<&Vec<IndexLabel>> = FxHashSet::default();
        let mut positions = Vec::new();
        let mut key: Vec<IndexLabel> = Vec::with_capacity(self.nlevels());
        for row in 0..self.len() {
            key.clear();
            key.extend(self.levels.iter().map(|level| level[row].clone()));
            if let Some(matched) = drop_set.get(&key) {
                found.insert(*matched);
            } else {
                positions.push(row);
            }
        }
        if let Some(missing) = labels_to_drop.iter().find(|label| !found.contains(label)) {
            return Err(IndexError::InvalidArgument(format!(
                "tuple key not found: {:?}",
                missing
            )));
        }
        Ok(self.take_existing_positions(&positions))
    }

    fn validate_key_arity(
        &self,
        key: &[IndexLabel],
        allow_partial: bool,
    ) -> Result<(), IndexError> {
        let nlevels = self.nlevels();
        if key.is_empty() {
            return Err(IndexError::InvalidArgument(
                "MultiIndex key must contain at least one level".to_owned(),
            ));
        }
        if (!allow_partial && key.len() != nlevels) || (allow_partial && key.len() > nlevels) {
            return Err(IndexError::InvalidArgument(format!(
                "wrong tuple arity: expected {}{}, got {}",
                if allow_partial { "1.." } else { "" },
                nlevels,
                key.len()
            )));
        }
        Ok(())
    }

    fn row_matches_prefix(&self, row: usize, key: &[IndexLabel]) -> bool {
        key.iter()
            .enumerate()
            .all(|(level, expected)| &self.levels[level][row] == expected)
    }

    fn row_prefix_cmp(&self, row: usize, key: &[IndexLabel]) -> std::cmp::Ordering {
        for (level, expected) in key.iter().enumerate() {
            let ord = self.levels[level][row].cmp(expected);
            if ord != std::cmp::Ordering::Equal {
                return ord;
            }
        }
        std::cmp::Ordering::Equal
    }

    /// Return matching row positions for an exact tuple key.
    pub fn get_loc_tuple(&self, key: &[IndexLabel]) -> Result<Vec<usize>, IndexError> {
        self.validate_key_arity(key, false)?;
        let positions: Vec<usize> = (0..self.len())
            .filter(|&row| self.row_matches_prefix(row, key))
            .collect();
        if positions.is_empty() {
            return Err(IndexError::InvalidArgument(format!(
                "tuple key not found: {:?}",
                key
            )));
        }
        Ok(positions)
    }

    /// Return row positions for an exact tuple, partial-prefix tuple, or a single level key.
    ///
    /// `level=None` treats `key` as an exact tuple when it has full arity, or a
    /// prefix tuple when shorter than `nlevels()`. `level=Some(n)` treats
    /// `key` as a single label lookup on that level.
    pub fn get_loc(
        &self,
        key: &[IndexLabel],
        level: Option<usize>,
    ) -> Result<Vec<usize>, IndexError> {
        if let Some(level) = level {
            if level >= self.nlevels() {
                return Err(IndexError::OutOfBounds {
                    position: level,
                    length: self.nlevels(),
                });
            }
            if key.len() != 1 {
                return Err(IndexError::InvalidArgument(format!(
                    "level lookup expects exactly one label, got {}",
                    key.len()
                )));
            }
            let positions: Vec<usize> = self.levels[level]
                .iter()
                .enumerate()
                .filter_map(|(row, label)| if label == &key[0] { Some(row) } else { None })
                .collect();
            if positions.is_empty() {
                return Err(IndexError::InvalidArgument(format!(
                    "level key not found at level {level}: {:?}",
                    key[0]
                )));
            }
            return Ok(positions);
        }

        self.validate_key_arity(key, true)?;
        let positions: Vec<usize> = (0..self.len())
            .filter(|&row| self.row_matches_prefix(row, key))
            .collect();
        if positions.is_empty() {
            return Err(IndexError::InvalidArgument(format!(
                "tuple key not found: {:?}",
                key
            )));
        }
        Ok(positions)
    }

    /// Return row positions for a list-like exact or partial-prefix selector.
    ///
    /// Matches the list-label subset of `pd.MultiIndex.get_locs(seq)`.
    pub fn get_locs(&self, key: &[IndexLabel]) -> Result<Vec<usize>, IndexError> {
        if key.is_empty() {
            return Ok(Vec::new());
        }
        self.get_loc(key, None)
    }

    /// pandas-style partial tuple lookup returning matching positions and the remaining index.
    pub fn get_loc_level(
        &self,
        key: &[IndexLabel],
    ) -> Result<(Vec<usize>, Option<MultiIndexOrIndex>), IndexError> {
        let positions = self.get_loc(key, None)?;
        if key.len() == self.nlevels() {
            return Ok((positions, None));
        }

        let mut remaining = MultiIndexOrIndex::Multi(self.take(&positions)?);
        for _ in 0..key.len() {
            remaining = match remaining {
                MultiIndexOrIndex::Multi(mi) => mi.droplevel(0)?,
                MultiIndexOrIndex::Index(_) => {
                    return Err(IndexError::InvalidArgument(
                        "cannot drop more levels than remain".to_owned(),
                    ));
                }
            };
        }

        Ok((positions, Some(remaining)))
    }

    /// Return `(start, stop)` bounds for a lexicographic tuple slice.
    ///
    /// The returned `stop` is exclusive, matching pandas `slice_locs`.
    pub fn slice_locs(
        &self,
        start: Option<&[IndexLabel]>,
        end: Option<&[IndexLabel]>,
    ) -> Result<(usize, usize), IndexError> {
        if let Some(start) = start {
            self.validate_key_arity(start, true)?;
        }
        if let Some(end) = end {
            self.validate_key_arity(end, true)?;
        }

        let start_pos = match start {
            Some(start_key) => (0..self.len())
                .find(|&row| self.row_prefix_cmp(row, start_key) != std::cmp::Ordering::Less)
                .unwrap_or(self.len()),
            None => 0,
        };
        let end_pos = match end {
            Some(end_key) => (0..self.len())
                .rfind(|&row| self.row_prefix_cmp(row, end_key) != std::cmp::Ordering::Greater)
                .map_or(0, |row| row + 1),
            None => self.len(),
        };

        if end_pos < start_pos {
            return Ok((start_pos, start_pos));
        }
        Ok((start_pos, end_pos))
    }

    /// Bound for a tuple slice, matching `pd.MultiIndex.get_slice_bound`.
    pub fn get_slice_bound(&self, label: &[IndexLabel], side: &str) -> Result<usize, IndexError> {
        match side {
            "left" => Ok(self.slice_locs(Some(label), Some(label))?.0),
            "right" => Ok(self.slice_locs(Some(label), Some(label))?.1),
            other => Err(IndexError::InvalidArgument(format!(
                "get_slice_bound: side must be 'left' or 'right', got {other:?}"
            ))),
        }
    }

    /// Alias for `slice_locs`, matching `pd.MultiIndex.slice_indexer`.
    pub fn slice_indexer(
        &self,
        start: Option<&[IndexLabel]>,
        end: Option<&[IndexLabel]>,
    ) -> Result<(usize, usize), IndexError> {
        self.slice_locs(start, end)
    }

    /// Return a label-bounded range, matching `pd.MultiIndex.truncate`.
    ///
    /// Bounds are interpreted as partial or full tuple prefixes and are
    /// inclusive on both sides. Open-ended bounds retain the corresponding
    /// leading or trailing rows.
    pub fn truncate(
        &self,
        before: Option<&[IndexLabel]>,
        after: Option<&[IndexLabel]>,
    ) -> Result<Self, IndexError> {
        let (start, stop) = self.slice_locs(before, after)?;
        let positions: Vec<usize> = (start..stop).collect();
        Ok(self.take_existing_positions(&positions))
    }

    /// Insertion positions for target tuples, matching `pd.MultiIndex.searchsorted`.
    ///
    /// `side` is `"left"` for the first valid insertion position or `"right"`
    /// for the position after an equal run. Like pandas, callers are expected
    /// to use this on lexicographically sorted indexes.
    pub fn searchsorted(&self, target: &Self, side: &str) -> Result<Vec<usize>, IndexError> {
        if side != "left" && side != "right" {
            return Err(IndexError::InvalidArgument(format!(
                "searchsorted: side must be 'left' or 'right', got {side:?}"
            )));
        }

        Ok((0..target.len())
            .map(|target_row| {
                let needle = target.tuple_at(target_row);
                let mut lo = 0_usize;
                let mut hi = self.len();
                while lo < hi {
                    let mid = lo + (hi - lo) / 2;
                    let cmp = self.tuple_at(mid).cmp(&needle);
                    use std::cmp::Ordering;
                    let go_right = matches!(
                        (cmp, side),
                        (Ordering::Less, _) | (Ordering::Equal, "right")
                    );
                    if go_right {
                        lo = mid + 1;
                    } else {
                        hi = mid;
                    }
                }
                lo
            })
            .collect())
    }

    /// Compute a non-unique indexer against another MultiIndex.
    ///
    /// Matches `pd.MultiIndex.get_indexer_non_unique(target)` by expanding
    /// every matching source position for each target tuple in target order.
    /// Missing target tuples contribute a single `-1` entry and their target
    /// position is recorded in the returned `missing` vector.
    #[must_use]
    /// Dictionary-encode every level of `self` and `target` into integer codes
    /// (consistent across both) and pack each row's tuple into one mixed-radix
    /// `u64` key (br-frankenpandas-mipack). Lets get_indexer hash an integer per
    /// row instead of allocating a `Vec<IndexLabel>` (and cloning Utf8 Strings)
    /// per row. Returns `None` when there are no levels, the level counts
    /// differ, or the combined code space overflows `u64` (caller keeps the
    /// `Vec<IndexLabel>`-key path). Bijective on tuple identity, so the source
    /// map and target lookups match exactly the same rows.
    /// Pack each row's tuple into one mixed-radix `u64` whose ascending order
    /// equals the lexicographic `row_cmp` order (br-frankenpandas-misort): per
    /// level, distinct values are ranked by `IndexLabel::Ord` and the rank codes
    /// are packed most-significant-first. So sorting these `u64` keys reproduces
    /// the level-by-level tuple sort while comparing integers instead of
    /// (Utf8) tuples. Returns `None` when there are no levels or the combined
    /// code space overflows `u64` (caller keeps the tuple-comparison sort).
    fn sorted_packed_keys(&self) -> Option<Vec<u64>> {
        let nlev = self.nlevels();
        if nlev == 0 {
            return None;
        }
        let n = self.len();
        let mut keys = vec![0u64; n];
        let mut combined: u128 = 1;
        for level in 0..nlev {
            let col = &self.levels[level];
            // Dedup to DISTINCT values first (O(n) hash), then sort only those
            // (O(d log d)); sorting all n refs would cost as much as the tuple
            // sort we are replacing.
            let mut sorted: Vec<&IndexLabel> =
                col.iter().collect::<FxHashSet<_>>().into_iter().collect();
            sorted.sort_unstable();
            let radix = sorted.len() as u64;
            let mut rank: FxHashMap<&IndexLabel, u64> =
                FxHashMap::with_capacity_and_hasher(sorted.len(), Default::default());
            for (r, value) in sorted.iter().enumerate() {
                rank.insert(*value, r as u64);
            }
            for (dst, value) in keys.iter_mut().zip(col.iter()) {
                *dst = dst.checked_mul(radix)?.checked_add(rank[value])?;
            }
            combined = combined.checked_mul(radix as u128)?;
            if combined > u64::MAX as u128 {
                return None;
            }
        }
        Some(keys)
    }

    /// Pack each row's tuple into one mixed-radix `u64` using FIRST-SEEN per-level
    /// codes (br-frankenpandas-midedup). Unlike [`Self::sorted_packed_keys`] this
    /// skips the per-level distinct sort — dedup only needs the keys to be a
    /// bijection on tuple identity, not lexicographically ordered. Lets
    /// duplicated/unique/drop_duplicates hash one integer per row instead of
    /// allocating (and Utf8-cloning) a `Vec<IndexLabel>` per row. `None` when
    /// there are no levels or the combined code space overflows `u64`.
    fn identity_packed_keys(&self) -> Option<Vec<u64>> {
        let nlev = self.nlevels();
        if nlev == 0 {
            return None;
        }
        let n = self.len();
        let mut keys = vec![0u64; n];
        let mut combined: u128 = 1;
        for level in 0..nlev {
            let col = &self.levels[level];
            let mut code: FxHashMap<&IndexLabel, u64> =
                FxHashMap::with_capacity_and_hasher(col.len(), Default::default());
            let mut next = 0u64;
            let codes: Vec<u64> = col
                .iter()
                .map(|value| {
                    *code.entry(value).or_insert_with(|| {
                        let c = next;
                        next += 1;
                        c
                    })
                })
                .collect();
            let radix = next;
            for (dst, &c) in keys.iter_mut().zip(&codes) {
                *dst = dst.checked_mul(radix)?.checked_add(c)?;
            }
            combined = combined.checked_mul(radix as u128)?;
            if combined > u64::MAX as u128 {
                return None;
            }
        }
        Some(keys)
    }

    fn factorize_packed_keys(&self, target: &Self) -> Option<(Vec<u64>, Vec<u64>)> {
        let nlev = self.nlevels();
        if nlev == 0 || nlev != target.nlevels() {
            return None;
        }
        let n = self.len();
        let m = target.len();
        let mut src = vec![0u64; n];
        let mut tgt = vec![0u64; m];
        let mut combined: u128 = 1;
        for level in 0..nlev {
            let mut codes: FxHashMap<&IndexLabel, u64> = FxHashMap::default();
            let mut next = 0u64;
            // Source first so its codes are dense and lookups stay consistent;
            // target-only values get fresh codes that no source key can match.
            let s_level = &self.levels[level];
            let t_level = &target.levels[level];
            let s_codes: Vec<u64> = (0..n)
                .map(|row| {
                    *codes.entry(&s_level[row]).or_insert_with(|| {
                        let c = next;
                        next += 1;
                        c
                    })
                })
                .collect();
            let t_codes: Vec<u64> = (0..m)
                .map(|row| {
                    *codes.entry(&t_level[row]).or_insert_with(|| {
                        let c = next;
                        next += 1;
                        c
                    })
                })
                .collect();
            // Mixed-radix: shift existing partial keys up by this level's radix
            // (= its distinct-value count) and add the new codes.
            let radix = next;
            for (dst, &c) in src.iter_mut().zip(&s_codes) {
                *dst = dst.checked_mul(radix)?.checked_add(c)?;
            }
            for (dst, &c) in tgt.iter_mut().zip(&t_codes) {
                *dst = dst.checked_mul(radix)?.checked_add(c)?;
            }
            combined = combined.checked_mul(radix as u128)?;
            if combined > u64::MAX as u128 {
                return None;
            }
        }
        Some((src, tgt))
    }

    pub fn get_indexer_non_unique(&self, target: &Self) -> (Vec<isize>, Vec<usize>) {
        if self.nlevels() != target.nlevels() {
            return (vec![-1; target.len()], (0..target.len()).collect());
        }

        if let Some((src_keys, tgt_keys)) = self.factorize_packed_keys(target) {
            let mut positions = FxHashMap::<u64, Vec<usize>>::with_capacity_and_hasher(
                self.len(),
                Default::default(),
            );
            for (row, &key) in src_keys.iter().enumerate() {
                positions.entry(key).or_default().push(row);
            }
            let mut indexer = Vec::new();
            let mut missing = Vec::new();
            for (target_row, &key) in tgt_keys.iter().enumerate() {
                if let Some(matches) = positions.get(&key) {
                    indexer.extend(matches.iter().map(|&pos| pos as isize));
                } else {
                    indexer.push(-1);
                    missing.push(target_row);
                }
            }
            return (indexer, missing);
        }

        let mut positions = FxHashMap::<Vec<IndexLabel>, Vec<usize>>::with_capacity_and_hasher(
            self.len(),
            Default::default(),
        );
        for row in 0..self.len() {
            let key: Vec<IndexLabel> = self.levels.iter().map(|level| level[row].clone()).collect();
            positions.entry(key).or_default().push(row);
        }

        let mut indexer = Vec::new();
        let mut missing = Vec::new();
        for target_row in 0..target.len() {
            let key: Vec<IndexLabel> = target
                .levels
                .iter()
                .map(|level| level[target_row].clone())
                .collect();
            if let Some(matches) = positions.get(&key) {
                indexer.extend(matches.iter().map(|&pos| pos as isize));
            } else {
                indexer.push(-1);
                missing.push(target_row);
            }
        }

        (indexer, missing)
    }

    /// Compute a positional indexer against another MultiIndex.
    ///
    /// Matches `pd.MultiIndex.get_indexer(target)` for unique source indexes:
    /// each target tuple maps to its first source position, and missing target
    /// tuples map to `-1`. Duplicate source tuples are rejected because pandas
    /// treats reindexing from a non-unique index as invalid; callers that want
    /// duplicate expansion should use [`Self::get_indexer_for`] or
    /// [`Self::get_indexer_non_unique`].
    pub fn get_indexer(&self, target: &Self) -> Result<Vec<isize>, IndexError> {
        if self.has_duplicates() {
            return Err(IndexError::InvalidArgument(
                "get_indexer requires a uniquely valued MultiIndex".to_owned(),
            ));
        }
        if self.nlevels() != target.nlevels() {
            return Ok(vec![-1; target.len()]);
        }

        if let Some((src_keys, tgt_keys)) = self.factorize_packed_keys(target) {
            let mut positions =
                FxHashMap::<u64, isize>::with_capacity_and_hasher(self.len(), Default::default());
            for (row, &key) in src_keys.iter().enumerate() {
                positions
                    .entry(key)
                    .or_insert(isize::try_from(row).unwrap_or(isize::MAX));
            }
            return Ok(tgt_keys
                .iter()
                .map(|key| positions.get(key).copied().unwrap_or(-1))
                .collect());
        }

        let mut positions = FxHashMap::<Vec<IndexLabel>, isize>::with_capacity_and_hasher(
            self.len(),
            Default::default(),
        );
        for row in 0..self.len() {
            positions
                .entry(self.tuple_at(row))
                .or_insert(isize::try_from(row).unwrap_or(isize::MAX));
        }

        Ok((0..target.len())
            .map(|target_row| {
                let key = target.tuple_at(target_row);
                positions.get(&key).copied().unwrap_or(-1)
            })
            .collect())
    }

    /// Compute a positional indexer, expanding duplicate source matches.
    ///
    /// Matches `pd.MultiIndex.get_indexer_for(target)`: unique source indexes
    /// use the compact one-position-per-target form, while non-unique source
    /// indexes expand every matching source position for each target tuple.
    pub fn get_indexer_for(&self, target: &Self) -> Result<Vec<isize>, IndexError> {
        if self.has_duplicates() {
            Ok(self.get_indexer_non_unique(target).0)
        } else {
            self.get_indexer(target)
        }
    }

    /// Reindex to a target MultiIndex, returning the target and source positions.
    ///
    /// Matches `pd.MultiIndex.reindex(target)` for unique source indexes:
    /// the returned index is the requested target, and the indexer maps each
    /// target tuple to its source position or `-1` for missing tuples.
    pub fn reindex(&self, target: &Self) -> Result<(Self, Vec<isize>), IndexError> {
        Ok((target.clone(), self.get_indexer(target)?))
    }

    /// Per-row flag for duplicated composite tuples.
    ///
    /// Matches `pd.MultiIndex.duplicated(keep='first'|'last'|False)`.
    /// - `DuplicateKeep::First` marks all but the first occurrence of each
    ///   tuple as duplicated (pandas default).
    /// - `DuplicateKeep::Last` marks all but the last occurrence.
    /// - `DuplicateKeep::None` marks every occurrence of any tuple that
    ///   appears more than once.
    #[must_use]
    pub fn duplicated(&self, keep: DuplicateKeep) -> Vec<bool> {
        let len = self.len();
        let mut out = vec![false; len];
        if len == 0 {
            return out;
        }
        // Materialize each row's composite key exactly once per pass. The prior
        // version built BOTH a counts and a first_seen map for every keep mode
        // (incl. a key.clone()) and then rebuilt the key again in the keep-mode
        // loop — 3-4 Vec<IndexLabel> allocations per row. Each mode now does the
        // minimal work; output is positional so marking order is irrelevant.
        // Packed-key fast path (br-frankenpandas-midedup): one u64 per row keyed
        // on tuple identity, so dedup hashes integers instead of allocating (and
        // Utf8-cloning) a Vec<IndexLabel> per row. Bijective on identity ⇒ the
        // dup mask is identical to the Vec-key path.
        if let Some(keys) = self.identity_packed_keys() {
            match keep {
                DuplicateKeep::First => {
                    let mut seen: FxHashSet<u64> =
                        FxHashSet::with_capacity_and_hasher(len, Default::default());
                    for (row, slot) in out.iter_mut().enumerate() {
                        if !seen.insert(keys[row]) {
                            *slot = true;
                        }
                    }
                }
                DuplicateKeep::Last => {
                    let mut seen: FxHashSet<u64> =
                        FxHashSet::with_capacity_and_hasher(len, Default::default());
                    for row in (0..len).rev() {
                        if !seen.insert(keys[row]) {
                            out[row] = true;
                        }
                    }
                }
                DuplicateKeep::None => {
                    let mut counts: FxHashMap<u64, usize> =
                        FxHashMap::with_capacity_and_hasher(len, Default::default());
                    for &key in &keys {
                        *counts.entry(key).or_insert(0) += 1;
                    }
                    for (row, slot) in out.iter_mut().enumerate() {
                        if counts[&keys[row]] > 1 {
                            *slot = true;
                        }
                    }
                }
            }
            return out;
        }

        let key_at = |row: usize| -> Vec<IndexLabel> {
            self.levels.iter().map(|level| level[row].clone()).collect()
        };
        match keep {
            DuplicateKeep::First => {
                // First occurrence kept; a key already seen earlier is a dup.
                let mut seen: FxHashSet<Vec<IndexLabel>> =
                    FxHashSet::with_capacity_and_hasher(len, Default::default());
                for (row, slot) in out.iter_mut().enumerate() {
                    if !seen.insert(key_at(row)) {
                        *slot = true;
                    }
                }
            }
            DuplicateKeep::Last => {
                // Last occurrence kept; scanning in reverse, the first key seen
                // in reverse is the last forward occurrence (kept = false).
                let mut seen: FxHashSet<Vec<IndexLabel>> =
                    FxHashSet::with_capacity_and_hasher(len, Default::default());
                for row in (0..len).rev() {
                    if !seen.insert(key_at(row)) {
                        out[row] = true;
                    }
                }
            }
            DuplicateKeep::None => {
                // Every occurrence of a key with count > 1 is a dup.
                let mut counts: FxHashMap<Vec<IndexLabel>, usize> =
                    FxHashMap::with_capacity_and_hasher(len, Default::default());
                for row in 0..len {
                    *counts.entry(key_at(row)).or_insert(0) += 1;
                }
                for (row, slot) in out.iter_mut().enumerate() {
                    if counts[&key_at(row)] > 1 {
                        *slot = true;
                    }
                }
            }
        }
        out
    }

    /// Whether all composite tuples are unique.
    ///
    /// Matches `pd.MultiIndex.is_unique`.
    #[must_use]
    pub fn is_unique(&self) -> bool {
        !self.duplicated(DuplicateKeep::First).iter().any(|&b| b)
    }

    /// Whether any composite tuple appears more than once.
    ///
    /// Matches `pd.MultiIndex.has_duplicates`.
    #[must_use]
    pub fn has_duplicates(&self) -> bool {
        !self.is_unique()
    }

    /// Drop duplicated tuples with pandas' default `keep='first'`.
    #[must_use]
    pub fn drop_duplicates(&self) -> Self {
        self.drop_duplicates_keep(DuplicateKeep::First)
    }

    /// Drop duplicated tuples with explicit keep behavior.
    #[must_use]
    pub fn drop_duplicates_keep(&self, keep: DuplicateKeep) -> Self {
        let duplicated = self.duplicated(keep);
        let positions: Vec<usize> = duplicated
            .iter()
            .enumerate()
            .filter_map(|(position, is_duplicated)| (!is_duplicated).then_some(position))
            .collect();
        self.take_existing_positions(&positions)
    }

    /// Unique tuples, preserving first-seen order.
    #[must_use]
    pub fn unique(&self) -> Self {
        self.drop_duplicates_keep(DuplicateKeep::First)
    }

    /// Number of unique tuples.
    #[must_use]
    pub fn nunique(&self) -> usize {
        self.unique().len()
    }

    /// Unsupported boolean reduction, matching `pd.MultiIndex.all()`.
    pub fn all(&self) -> Result<bool, IndexError> {
        Err(IndexError::InvalidArgument(
            "cannot perform all with this index type: MultiIndex".to_owned(),
        ))
    }

    /// Unsupported boolean reduction, matching `pd.MultiIndex.any()`.
    pub fn any(&self) -> Result<bool, IndexError> {
        Err(IndexError::InvalidArgument(
            "cannot perform any with this index type: MultiIndex".to_owned(),
        ))
    }

    /// Factorize tuples into integer codes and unique tuples.
    ///
    /// Missing labels remain part of the composite tuple identity, matching
    /// pandas' MultiIndex-level factorization behavior.
    #[must_use]
    pub fn factorize(&self) -> (Vec<isize>, Self) {
        let mut positions = HashMap::<Vec<IndexLabel>, isize>::new();
        let mut uniques = Vec::<Vec<IndexLabel>>::new();
        let mut codes = Vec::with_capacity(self.len());
        for tuple in self.to_list() {
            if let Some(code) = positions.get(&tuple) {
                codes.push(*code);
            } else {
                let code = isize::try_from(uniques.len()).unwrap_or(isize::MAX);
                positions.insert(tuple.clone(), code);
                uniques.push(tuple);
                codes.push(code);
            }
        }
        let mut levels: Vec<Vec<IndexLabel>> = (0..self.nlevels())
            .map(|_| Vec::with_capacity(uniques.len()))
            .collect();
        for tuple in uniques {
            for (level_idx, label) in tuple.into_iter().enumerate() {
                levels[level_idx].push(label);
            }
        }
        let unique_index = Self {
            levels,
            names: self.names.clone(),
        };
        (codes, unique_index)
    }

    /// Count unique tuple occurrences, sorted by count descending then tuple.
    #[must_use]
    pub fn value_counts(&self) -> Vec<(Vec<IndexLabel>, usize)> {
        let mut counts = HashMap::<Vec<IndexLabel>, usize>::new();
        for tuple in self.to_list() {
            *counts.entry(tuple).or_insert(0) += 1;
        }
        let mut pairs: Vec<(Vec<IndexLabel>, usize)> = counts.into_iter().collect();
        pairs.sort_by(|(left_tuple, left_count), (right_tuple, right_count)| {
            right_count
                .cmp(left_count)
                .then_with(|| left_tuple.cmp(right_tuple))
        });
        pairs
    }

    /// Positional sorter for lexicographic tuple order.
    #[must_use]
    pub fn argsort(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.len()).collect();
        // Packed-key fast path: sort on one u64 per row (ascending u64 order ==
        // lexicographic row_cmp order) instead of comparing (Utf8) tuples. The
        // `.then(left.cmp(right))` original-position tiebreak is preserved, so
        // the permutation is identical to the tuple-comparison sort.
        if let Some(keys) = self.sorted_packed_keys() {
            order.sort_by(|&left, &right| {
                keys[left].cmp(&keys[right]).then_with(|| left.cmp(&right))
            });
            return order;
        }
        order.sort_by(|&left, &right| self.row_cmp(left, right).then_with(|| left.cmp(&right)));
        order
    }

    /// Sort tuples lexicographically, matching `pd.MultiIndex.sort_values()`.
    #[must_use]
    pub fn sort_values(&self) -> Self {
        self.take_existing_positions(&self.argsort())
    }

    /// Alias for `sort_values`, matching `pd.MultiIndex.sort`.
    #[must_use]
    pub fn sort(&self) -> Self {
        self.sort_values()
    }

    /// Sort tuples and return the positional indexer used for the sort.
    #[must_use]
    pub fn sortlevel(&self) -> (Self, Vec<usize>) {
        let order = self.argsort();
        (self.take_existing_positions(&order), order)
    }

    /// Lexicographic minimum tuple.
    #[must_use]
    pub fn min(&self) -> Option<Vec<IndexLabel>> {
        self.argsort()
            .first()
            .map(|&position| self.tuple_at(position))
    }

    /// Lexicographic maximum tuple.
    #[must_use]
    pub fn max(&self) -> Option<Vec<IndexLabel>> {
        self.argsort()
            .last()
            .map(|&position| self.tuple_at(position))
    }

    /// Position of the maximum tuple.
    #[must_use]
    pub fn argmax(&self) -> Option<usize> {
        self.argsort().last().copied()
    }

    /// Position of the minimum tuple.
    #[must_use]
    pub fn argmin(&self) -> Option<usize> {
        self.argsort().first().copied()
    }

    /// Append another MultiIndex to this one.
    ///
    /// Matches `pd.MultiIndex.append(other)` for equal-level indexes.
    pub fn append(&self, other: &Self) -> Result<Self, IndexError> {
        self.ensure_same_nlevels(other)?;
        let mut levels = Vec::with_capacity(self.nlevels());
        for level_idx in 0..self.nlevels() {
            let mut level = self.levels[level_idx].clone();
            level.extend(other.levels[level_idx].iter().cloned());
            levels.push(level);
        }
        Ok(Self {
            levels,
            names: self.shared_names(other),
        })
    }

    /// Repeat each tuple `repeats` times, matching `pd.MultiIndex.repeat`.
    #[must_use]
    pub fn repeat(&self, repeats: usize) -> Self {
        if repeats == 1 {
            return self.clone();
        }
        let mut levels = Vec::with_capacity(self.nlevels());
        for level in &self.levels {
            let mut repeated = Vec::with_capacity(level.len() * repeats);
            for label in level {
                for _ in 0..repeats {
                    repeated.push(label.clone());
                }
            }
            levels.push(repeated);
        }
        Self {
            levels,
            names: self.names.clone(),
        }
    }

    /// Drop tuples containing any missing level label.
    ///
    /// Matches `pd.MultiIndex.dropna(how='any')`, which is pandas' default.
    #[must_use]
    pub fn dropna(&self) -> Self {
        self.dropna_any()
    }

    /// Drop tuples containing any missing level label.
    #[must_use]
    pub fn dropna_any(&self) -> Self {
        let positions: Vec<usize> = (0..self.len())
            .filter(|&row| self.levels.iter().all(|level| !level[row].is_missing()))
            .collect();
        self.take_existing_positions(&positions)
    }

    /// Drop tuples whose level labels are all missing.
    #[must_use]
    pub fn dropna_all(&self) -> Self {
        let positions: Vec<usize> = (0..self.len())
            .filter(|&row| !self.levels.iter().all(|level| level[row].is_missing()))
            .collect();
        self.take_existing_positions(&positions)
    }

    /// Tuple intersection preserving left order and de-duplicating results.
    pub fn intersection(&self, other: &Self) -> Result<Self, IndexError> {
        self.ensure_same_nlevels(other)?;
        // Packed-key fast path (br-frankenpandas-misetop): identity-coded u64 per
        // row instead of to_list() (per-row Vec<IndexLabel> + Utf8 clone) and a
        // SipHash HashMap<Vec<IndexLabel>>. Keep self rows whose key is in other,
        // deduped first-seen, then gather those positions. Bijective on tuple
        // identity ⇒ same kept rows, same order.
        if let Some((self_keys, other_keys)) = self.factorize_packed_keys(other) {
            let other_set: FxHashSet<u64> = other_keys.into_iter().collect();
            let mut seen: FxHashSet<u64> =
                FxHashSet::with_capacity_and_hasher(self_keys.len(), Default::default());
            let positions: Vec<usize> = self_keys
                .iter()
                .enumerate()
                .filter_map(|(i, &k)| (other_set.contains(&k) && seen.insert(k)).then_some(i))
                .collect();
            return Ok(self
                .take_existing_positions(&positions)
                .set_names(self.shared_names(other)));
        }
        let other_keys: HashMap<Vec<IndexLabel>, ()> = other
            .to_list()
            .into_iter()
            .map(|tuple| (tuple, ()))
            .collect();
        let mut seen = HashMap::<Vec<IndexLabel>, ()>::new();
        let tuples = self
            .to_list()
            .into_iter()
            .filter(|tuple| {
                other_keys.contains_key(tuple) && seen.insert(tuple.clone(), ()).is_none()
            })
            .collect();
        Self::from_tuples_with_names(tuples, self.shared_names(other))
    }

    /// Tuple union preserving first-seen order from `self` then `other`.
    pub fn union(&self, other: &Self) -> Result<Self, IndexError> {
        self.ensure_same_nlevels(other)?;
        let mut seen = HashMap::<Vec<IndexLabel>, ()>::new();
        let mut tuples = Vec::with_capacity(self.len() + other.len());
        for tuple in self.to_list().into_iter().chain(other.to_list()) {
            if seen.insert(tuple.clone(), ()).is_none() {
                tuples.push(tuple);
            }
        }
        Self::from_tuples_with_names(tuples, self.shared_names(other))
    }

    /// Alias for `union`, matching the flat `Index::union_with` naming.
    pub fn union_with(&self, other: &Self) -> Result<Self, IndexError> {
        self.union(other)
    }

    /// Tuple difference preserving left order and de-duplicating results.
    pub fn difference(&self, other: &Self) -> Result<Self, IndexError> {
        self.ensure_same_nlevels(other)?;
        // Packed-key fast path (br-frankenpandas-misetop): keep self rows whose
        // key is NOT in other, deduped first-seen. See intersection.
        if let Some((self_keys, other_keys)) = self.factorize_packed_keys(other) {
            let other_set: FxHashSet<u64> = other_keys.into_iter().collect();
            let mut seen: FxHashSet<u64> =
                FxHashSet::with_capacity_and_hasher(self_keys.len(), Default::default());
            let positions: Vec<usize> = self_keys
                .iter()
                .enumerate()
                .filter_map(|(i, &k)| (!other_set.contains(&k) && seen.insert(k)).then_some(i))
                .collect();
            return Ok(self
                .take_existing_positions(&positions)
                .set_names(self.shared_names(other)));
        }
        let other_keys: HashMap<Vec<IndexLabel>, ()> = other
            .to_list()
            .into_iter()
            .map(|tuple| (tuple, ()))
            .collect();
        let mut seen = HashMap::<Vec<IndexLabel>, ()>::new();
        let tuples = self
            .to_list()
            .into_iter()
            .filter(|tuple| {
                !other_keys.contains_key(tuple) && seen.insert(tuple.clone(), ()).is_none()
            })
            .collect();
        Self::from_tuples_with_names(tuples, self.shared_names(other))
    }

    /// Tuple symmetric difference preserving first-seen order.
    pub fn symmetric_difference(&self, other: &Self) -> Result<Self, IndexError> {
        self.ensure_same_nlevels(other)?;
        let self_keys: HashMap<Vec<IndexLabel>, ()> = self
            .to_list()
            .into_iter()
            .map(|tuple| (tuple, ()))
            .collect();
        let other_keys: HashMap<Vec<IndexLabel>, ()> = other
            .to_list()
            .into_iter()
            .map(|tuple| (tuple, ()))
            .collect();
        let mut seen = HashMap::<Vec<IndexLabel>, ()>::new();
        let mut tuples = Vec::new();
        for tuple in self.to_list() {
            if !other_keys.contains_key(&tuple) && seen.insert(tuple.clone(), ()).is_none() {
                tuples.push(tuple);
            }
        }
        for tuple in other.to_list() {
            if !self_keys.contains_key(&tuple) && seen.insert(tuple.clone(), ()).is_none() {
                tuples.push(tuple);
            }
        }
        Self::from_tuples_with_names(tuples, self.shared_names(other))
    }

    /// Group tuple positions by composite key, matching `pd.MultiIndex.groupby`.
    #[must_use]
    pub fn groupby(&self) -> HashMap<Vec<IndexLabel>, Vec<usize>> {
        let mut groups = HashMap::<Vec<IndexLabel>, Vec<usize>>::new();
        for row in 0..self.len() {
            groups.entry(self.tuple_at(row)).or_default().push(row);
        }
        groups
    }

    /// Join two MultiIndexes using pandas-style join modes.
    pub fn join(&self, other: &Self, how: &str) -> Result<Self, IndexError> {
        match how {
            "left" => Ok(self.clone()),
            "right" => Ok(other.clone()),
            "inner" => self.intersection(other),
            "outer" => self.union(other),
            other => Err(IndexError::InvalidArgument(format!(
                "join: how must be 'left', 'right', 'inner', or 'outer', got {other:?}"
            ))),
        }
    }

    /// Per-row membership test against a set of tuples.
    ///
    /// Matches `pd.MultiIndex.isin(values)`. Each entry in the returned
    /// bool vector is `true` iff that row's composite tuple appears in
    /// `values`. Tuples whose length does not match the MultiIndex's
    /// number of levels never match (silently contribute `false`),
    /// matching pandas' lenient behavior.
    #[must_use]
    pub fn isin(&self, values: &[Vec<IndexLabel>]) -> Vec<bool> {
        let nlevels = self.nlevels();
        let lookup: FxHashSet<&Vec<IndexLabel>> =
            values.iter().filter(|v| v.len() == nlevels).collect();
        if lookup.is_empty() {
            return vec![false; self.len()];
        }
        // Reuse one key buffer across rows: clear + extend refills the composite
        // lookup key in place, so membership is tested without allocating a fresh
        // Vec<IndexLabel> per row. Result is identical (value-based membership,
        // positional bool output).
        let mut key: Vec<IndexLabel> = Vec::with_capacity(nlevels);
        let mut out = Vec::with_capacity(self.len());
        for row in 0..self.len() {
            key.clear();
            key.extend(self.levels.iter().map(|level| level[row].clone()));
            out.push(lookup.contains(&key));
        }
        out
    }

    /// Per-row membership test against values for a single level.
    ///
    /// Matches `pd.MultiIndex.isin(values, level=...)`. Returns `true`
    /// for positions whose label at `level` is in `values`. Returns an
    /// `OutOfBounds` error when `level` exceeds `nlevels()`.
    pub fn isin_level(&self, values: &[IndexLabel], level: usize) -> Result<Vec<bool>, IndexError> {
        if level >= self.nlevels() {
            return Err(IndexError::OutOfBounds {
                position: level,
                length: self.nlevels(),
            });
        }
        let lookup: FxHashSet<&IndexLabel> = values.iter().collect();
        Ok(self.levels[level]
            .iter()
            .map(|label| lookup.contains(label))
            .collect())
    }

    /// Construct a MultiIndex from tuples of labels.
    ///
    /// Matches `pd.MultiIndex.from_tuples(tuples)`.
    /// Each inner Vec represents one row's labels across all levels.
    pub fn from_tuples(tuples: Vec<Vec<IndexLabel>>) -> Result<Self, IndexError> {
        if tuples.is_empty() {
            return Ok(Self {
                levels: Vec::new(),
                names: Vec::new(),
            });
        }

        let nlevels = tuples[0].len();
        for (i, t) in tuples.iter().enumerate() {
            if t.len() != nlevels {
                return Err(IndexError::LengthMismatch {
                    expected: nlevels,
                    actual: t.len(),
                    context: format!("tuple at position {i} has wrong number of levels"),
                });
            }
        }

        let mut levels: Vec<Vec<IndexLabel>> = (0..nlevels)
            .map(|_| Vec::with_capacity(tuples.len()))
            .collect();
        for tuple in &tuples {
            for (level_idx, label) in tuple.iter().enumerate() {
                levels[level_idx].push(label.clone());
            }
        }

        Ok(Self {
            levels,
            names: vec![None; nlevels],
        })
    }

    /// Construct a MultiIndex from parallel arrays (one per level).
    ///
    /// Matches `pd.MultiIndex.from_arrays(arrays)`.
    pub fn from_arrays(arrays: Vec<Vec<IndexLabel>>) -> Result<Self, IndexError> {
        if arrays.is_empty() {
            return Ok(Self {
                levels: Vec::new(),
                names: Vec::new(),
            });
        }

        let expected_len = arrays[0].len();
        for (i, arr) in arrays.iter().enumerate() {
            if arr.len() != expected_len {
                return Err(IndexError::LengthMismatch {
                    expected: expected_len,
                    actual: arr.len(),
                    context: format!("level {i} array length mismatch"),
                });
            }
        }

        let nlevels = arrays.len();
        Ok(Self {
            levels: arrays,
            names: vec![None; nlevels],
        })
    }

    /// Construct a MultiIndex from frame-like columns.
    ///
    /// Matches `pd.MultiIndex.from_frame(frame)` at the payload level:
    /// each input entry is one frame column, with the optional column name
    /// becoming the corresponding level name.
    pub fn from_frame(columns: Vec<(Option<String>, Vec<IndexLabel>)>) -> Result<Self, IndexError> {
        if columns.is_empty() {
            return Ok(Self {
                levels: Vec::new(),
                names: Vec::new(),
            });
        }

        let expected_len = columns[0].1.len();
        for (column_idx, (_, values)) in columns.iter().enumerate() {
            if values.len() != expected_len {
                return Err(IndexError::LengthMismatch {
                    expected: expected_len,
                    actual: values.len(),
                    context: format!("from_frame column {column_idx} length mismatch"),
                });
            }
        }

        let mut names = Vec::with_capacity(columns.len());
        let mut levels = Vec::with_capacity(columns.len());
        for (name, values) in columns {
            names.push(name);
            levels.push(values);
        }

        Ok(Self { levels, names })
    }

    /// Construct a MultiIndex from the Cartesian product of iterables.
    ///
    /// Matches `pd.MultiIndex.from_product(iterables)`.
    pub fn from_product(iterables: Vec<Vec<IndexLabel>>) -> Result<Self, IndexError> {
        if iterables.is_empty() {
            return Ok(Self {
                levels: Vec::new(),
                names: Vec::new(),
            });
        }

        // Compute total size of the Cartesian product.
        let total: usize = iterables.iter().map(Vec::len).product();
        if total == 0 {
            let nlevels = iterables.len();
            return Ok(Self {
                levels: (0..nlevels).map(|_| Vec::new()).collect(),
                names: vec![None; nlevels],
            });
        }

        let nlevels = iterables.len();
        let mut levels: Vec<Vec<IndexLabel>> =
            (0..nlevels).map(|_| Vec::with_capacity(total)).collect();

        // Generate Cartesian product: for each position, compute which
        // element from each level by dividing position by the product of
        // all subsequent level lengths.
        for pos in 0..total {
            let mut remaining = pos;
            for (level_idx, iterable) in iterables.iter().enumerate().rev() {
                let idx_in_level = remaining % iterable.len();
                remaining /= iterable.len();
                levels[level_idx].push(iterable[idx_in_level].clone());
            }
        }

        Ok(Self {
            levels,
            names: vec![None; nlevels],
        })
    }

    /// Flatten this MultiIndex into a single-level Index by joining
    /// level labels with a separator.
    ///
    /// Matches `pd.MultiIndex.to_flat_index()` (approximately).
    #[must_use]
    pub fn to_flat_index(&self, sep: &str) -> Index {
        let n = self.len();
        let labels: Vec<IndexLabel> = (0..n)
            .map(|i| {
                let parts: Vec<String> = self
                    .levels
                    .iter()
                    .map(|level| level[i].to_string())
                    .collect();
                IndexLabel::Utf8(parts.join(sep))
            })
            .collect();
        Index::new(labels)
    }

    /// Drop a level from this MultiIndex, returning a new MultiIndex
    /// (or an Index if only one level remains).
    ///
    /// Matches `pd.MultiIndex.droplevel(level)`.
    pub fn droplevel(&self, level: usize) -> Result<MultiIndexOrIndex, IndexError> {
        if level >= self.nlevels() {
            return Err(IndexError::OutOfBounds {
                position: level,
                length: self.nlevels(),
            });
        }
        if self.nlevels() <= 1 {
            return Err(IndexError::OutOfBounds {
                position: level,
                length: self.nlevels(),
            });
        }

        let mut new_levels = self.levels.clone();
        new_levels.remove(level);
        let mut new_names = self.names.clone();
        new_names.remove(level);

        if new_levels.len() == 1 {
            let mut idx = Index::new(new_levels.into_iter().next().unwrap());
            if let Some(ref name) = new_names[0] {
                idx = idx.set_name(name);
            }
            Ok(MultiIndexOrIndex::Index(idx))
        } else {
            Ok(MultiIndexOrIndex::Multi(Self {
                levels: new_levels,
                names: new_names,
            }))
        }
    }

    /// Swap two levels.
    ///
    /// Matches `pd.MultiIndex.swaplevel(i, j)`.
    pub fn swaplevel(&self, i: usize, j: usize) -> Result<Self, IndexError> {
        if i >= self.nlevels() || j >= self.nlevels() {
            return Err(IndexError::OutOfBounds {
                position: i.max(j),
                length: self.nlevels(),
            });
        }
        let mut new_levels = self.levels.clone();
        let mut new_names = self.names.clone();
        new_levels.swap(i, j);
        new_names.swap(i, j);
        Ok(Self {
            levels: new_levels,
            names: new_names,
        })
    }

    /// Reorder levels according to the given order.
    ///
    /// Matches `pd.MultiIndex.reorder_levels(order)`.
    /// `order` is a slice of level indices specifying the new order.
    /// Must contain each level index exactly once.
    pub fn reorder_levels(&self, order: &[usize]) -> Result<Self, IndexError> {
        if order.len() != self.nlevels() {
            return Err(IndexError::LengthMismatch {
                expected: self.nlevels(),
                actual: order.len(),
                context: "reorder_levels: order length must match nlevels".into(),
            });
        }

        // Validate all indices are in range and unique.
        let mut seen = vec![false; self.nlevels()];
        for &idx in order {
            if idx >= self.nlevels() {
                return Err(IndexError::OutOfBounds {
                    position: idx,
                    length: self.nlevels(),
                });
            }
            if seen[idx] {
                return Err(IndexError::LengthMismatch {
                    expected: self.nlevels(),
                    actual: order.len(),
                    context: format!("reorder_levels: duplicate level index {idx}"),
                });
            }
            seen[idx] = true;
        }

        let new_levels: Vec<Vec<IndexLabel>> =
            order.iter().map(|&idx| self.levels[idx].clone()).collect();
        let new_names: Vec<Option<String>> =
            order.iter().map(|&idx| self.names[idx].clone()).collect();

        Ok(Self {
            levels: new_levels,
            names: new_names,
        })
    }
}

/// Result of `MultiIndex::droplevel` — either a MultiIndex (if 2+ levels remain)
/// or a plain Index (if reduced to 1 level).
#[derive(Debug, Clone, PartialEq)]
pub enum MultiIndexOrIndex {
    Multi(MultiIndex),
    Index(Index),
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fp_types::{Period, PeriodFreq, Scalar, Timedelta};

    use super::{
        CategoricalIndex, DateOffset, DateRangeError, DatetimeIndex, Index, IndexLabel, MultiIndex,
        PeriodFields, PeriodIndex, RangeIndex, TimedeltaIndex, TimedeltaRangeError, align_union,
        apply_date_offset, bdate_range, date_range, infer_freq_from_timestamps, timedelta_range,
        validate_alignment_plan,
    };

    fn int64_labels(index: &Index) -> Vec<i64> {
        index
            .labels()
            .iter()
            .filter_map(|label| match label {
                IndexLabel::Int64(value) => Some(*value),
                _ => None,
            })
            .collect()
    }

    /// Regression lock for br-frankenpandas-i3t8. `Index` must stay
    /// `Send + Sync` so `DataFrame` can be wrapped in `Arc` and shared
    /// across reader threads. A future refactor that reintroduces
    /// `std::cell::OnceCell` (or any `!Sync` interior-mutability primitive)
    /// breaks this test at compile time.
    #[test]
    fn index_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Index>();
        assert_send_sync::<MultiIndex>();
    }

    #[test]
    fn bdate_range_rolls_weekend_start_forward() {
        let idx = bdate_range(Some("2024-01-06"), None, Some(3), None).unwrap();
        assert_eq!(
            idx.labels(),
            &[
                IndexLabel::Datetime64(1_704_672_000_000_000_000),
                IndexLabel::Datetime64(1_704_758_400_000_000_000),
                IndexLabel::Datetime64(1_704_844_800_000_000_000),
            ]
        );
    }

    #[test]
    fn bdate_range_rolls_weekend_end_backward_and_preserves_name() {
        let idx = bdate_range(None, Some("2024-01-07"), Some(3), Some("biz")).unwrap();
        assert_eq!(
            idx.labels(),
            &[
                IndexLabel::Datetime64(1_704_240_000_000_000_000),
                IndexLabel::Datetime64(1_704_326_400_000_000_000),
                IndexLabel::Datetime64(1_704_412_800_000_000_000),
            ]
        );
        assert_eq!(idx.name(), Some("biz"));
    }

    #[test]
    fn timedelta_range_rejects_over_specified_parameters() {
        let err = timedelta_range(
            Some(Timedelta::NANOS_PER_DAY),
            Some(3 * Timedelta::NANOS_PER_DAY),
            Some(2),
            Timedelta::NANOS_PER_DAY,
            None,
        )
        .expect_err("start + end + periods with explicit freq must fail closed");
        assert!(matches!(err, TimedeltaRangeError::TooManyParams));
    }

    #[test]
    fn date_range_rejects_over_specified_parameters() {
        let err = date_range(
            Some("2020-01-01"),
            Some("2020-01-03"),
            Some(2),
            Timedelta::NANOS_PER_DAY,
            None,
        )
        .expect_err("start + end + periods with explicit freq must fail closed");
        assert!(matches!(err, DateRangeError::TooManyParams));
    }

    #[test]
    fn date_range_rejects_generated_timestamp_overflow() {
        let err = date_range(
            Some("2262-04-11 23:47:16"),
            None,
            Some(3),
            Timedelta::NANOS_PER_SEC,
            None,
        )
        .expect_err("overflow past i64::MAX nanos must fail closed");
        assert!(matches!(err, DateRangeError::InvalidRange));
    }

    #[test]
    fn date_range_rejects_backfilled_timestamp_underflow() {
        let err = date_range(
            None,
            Some("1677-09-21 00:12:44"),
            Some(3),
            Timedelta::NANOS_PER_SEC,
            None,
        )
        .expect_err("underflow before i64::MIN nanos must fail closed");
        assert!(matches!(err, DateRangeError::InvalidRange));
    }

    #[test]
    fn date_range_rejects_out_of_bounds_timestamp_parse() {
        let err = date_range(
            Some("2263-01-01"),
            None,
            Some(1),
            Timedelta::NANOS_PER_DAY,
            None,
        )
        .expect_err("out-of-bounds timestamps must not be coerced to i64::MIN");
        assert!(matches!(err, DateRangeError::InvalidRange));
    }

    #[test]
    fn date_offset_business_day_skips_weekend() {
        let nanos = apply_date_offset("2024-01-05", DateOffset::BusinessDay(1)).unwrap();
        assert_eq!(nanos, 1_704_672_000_000_000_000);
    }

    #[test]
    fn date_offset_month_end_handles_leap_year() {
        let nanos = apply_date_offset("2024-02-10", DateOffset::MonthEnd(1)).unwrap();
        assert_eq!(nanos, 1_709_164_800_000_000_000);
    }

    #[test]
    fn infer_freq_detects_fixed_and_calendar_offsets() {
        assert_eq!(
            infer_freq_from_timestamps(&["2024-01-01", "2024-01-03", "2024-01-05"]).unwrap(),
            Some("2D".to_owned())
        );
        assert_eq!(
            infer_freq_from_timestamps(&[
                "2024-01-01",
                "2024-01-02",
                "2024-01-03",
                "2024-01-04",
                "2024-01-05",
                "2024-01-08",
                "2024-01-09",
            ])
            .unwrap(),
            Some("B".to_owned())
        );
        assert_eq!(
            infer_freq_from_timestamps(&["2024-01-31", "2024-02-29", "2024-03-31"]).unwrap(),
            Some("ME".to_owned())
        );
    }

    #[test]
    fn infer_freq_returns_none_for_irregular_or_duplicate_values() {
        assert_eq!(
            infer_freq_from_timestamps(&["2024-01-01", "2024-01-02", "2024-01-04"]).unwrap(),
            None
        );
        assert_eq!(
            infer_freq_from_timestamps(&["2024-01-01", "2024-01-02", "2024-01-02"]).unwrap(),
            None
        );
    }

    #[test]
    fn union_alignment_preserves_left_then_right_unseen_order() {
        let left = Index::new(vec![1_i64.into(), 2_i64.into(), 4_i64.into()]);
        let right = Index::new(vec![2_i64.into(), 3_i64.into(), 4_i64.into()]);

        let plan = align_union(&left, &right);
        assert_eq!(
            plan.union_index.labels(),
            &[
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
                IndexLabel::Int64(4),
                IndexLabel::Int64(3),
            ]
        );
        assert_eq!(plan.left_positions, vec![Some(0), Some(1), Some(2), None]);
        assert_eq!(plan.right_positions, vec![None, Some(0), Some(2), Some(1)]);
        validate_alignment_plan(&plan).expect("plan must be valid");
    }

    #[test]
    fn duplicate_detection_matches_index_surface() {
        let index = Index::new(vec!["a".into(), "a".into(), "b".into()]);
        assert!(index.has_duplicates());
    }

    #[test]
    fn has_duplicates_sort_fast_path_matches_hashmap_idxdup() {
        // The strict-ascending fast path in has_duplicates must agree with the
        // FxHashMap detect_duplicates for every shape: sorted-unique (fast path
        // returns false), sorted-with-dups (not strictly ascending -> Unsorted
        // -> hashmap), unsorted-unique, unsorted-dups, descending, single,
        // empty, and Utf8.
        let cases: Vec<Vec<IndexLabel>> = vec![
            vec![],
            vec![5_i64.into()],
            vec![1_i64.into(), 2_i64.into(), 3_i64.into()], // sorted unique
            vec![1_i64.into(), 5_i64.into(), 9_i64.into()], // sorted unique, gapped
            vec![1_i64.into(), 2_i64.into(), 2_i64.into()], // sorted with dup
            vec![3_i64.into(), 1_i64.into(), 2_i64.into()], // unsorted unique
            vec![3_i64.into(), 1_i64.into(), 3_i64.into()], // unsorted dup
            vec![9_i64.into(), 5_i64.into(), 1_i64.into()], // descending unique
            vec!["a".into(), "b".into(), "c".into()],       // sorted utf8 unique
            vec!["a".into(), "a".into(), "b".into()],       // utf8 dup
            vec!["c".into(), "a".into(), "b".into()],       // unsorted utf8 unique
        ];
        for labels in cases {
            let expected = super::detect_duplicates(&labels);
            let got = Index::new(labels.clone()).has_duplicates();
            assert_eq!(got, expected, "mismatch for {labels:?}");
        }
    }

    #[test]
    fn dedup_family_sort_fast_path_matches_reference_idxdup() {
        // unique / duplicated / drop_duplicates strict-ascending fast paths must
        // equal an independent first-seen reference for every shape.
        let cases: Vec<Vec<IndexLabel>> = vec![
            vec![],
            vec![7_i64.into()],
            vec![1_i64.into(), 2_i64.into(), 3_i64.into()], // sorted unique
            vec![1_i64.into(), 2_i64.into(), 2_i64.into(), 4_i64.into()], // sorted dup
            vec![3_i64.into(), 1_i64.into(), 3_i64.into(), 2_i64.into()], // unsorted dup
            vec![9_i64.into(), 5_i64.into(), 1_i64.into()], // descending
            vec!["a".into(), "b".into(), "c".into()],       // sorted utf8
            vec!["b".into(), "a".into(), "b".into()],       // unsorted utf8 dup
        ];
        for labels in cases {
            let idx = Index::new(labels.clone());

            let mut seen = std::collections::HashSet::new();
            let ref_unique: Vec<IndexLabel> = labels
                .iter()
                .filter(|l| seen.insert((*l).clone()))
                .cloned()
                .collect();
            assert_eq!(
                idx.unique().labels(),
                ref_unique.as_slice(),
                "unique {labels:?}"
            );

            let mut seen_f = std::collections::HashSet::new();
            let ref_dup_first: Vec<bool> =
                labels.iter().map(|l| !seen_f.insert(l.clone())).collect();
            assert_eq!(
                idx.duplicated(DuplicateKeep::First),
                ref_dup_first,
                "duplicated(First) {labels:?}"
            );

            assert_eq!(
                idx.drop_duplicates().labels(),
                ref_unique.as_slice(),
                "drop_duplicates {labels:?}"
            );
        }
    }

    #[test]
    fn sorted_merge_set_ops_match_reference_idxdup() {
        // intersection / difference must equal the self-ordered, deduped
        // FxHashMap reference whether the two-pointer fast path fires (both
        // strictly ascending) or the hash path runs (any side unsorted).
        let s = |v: &[i64]| v.iter().map(|x| IndexLabel::Int64(*x)).collect::<Vec<_>>();
        let pairs: Vec<(Vec<IndexLabel>, Vec<IndexLabel>)> = vec![
            (s(&[1, 2, 3, 5]), s(&[2, 3, 4])), // both sorted, overlap
            (s(&[1, 2, 3]), s(&[4, 5, 6])),    // both sorted, disjoint
            (s(&[1, 2, 3]), s(&[1, 2, 3])),    // identical
            (s(&[1, 2, 3]), vec![]),           // empty other
            (vec![], s(&[1, 2, 3])),           // empty self
            (s(&[3, 1, 2]), s(&[2, 3])),       // self unsorted -> hash path
            (s(&[1, 2, 3]), s(&[3, 1])),       // other unsorted -> hash path
            (
                vec!["a".into(), "c".into(), "e".into()],
                vec!["b".into(), "c".into()],
            ), // utf8 sorted
            (
                vec![1_i64.into(), 2_i64.into()],
                vec!["a".into(), "b".into()],
            ), // mixed-type sorted, disjoint by Ord variant
        ];
        for (a, b) in pairs {
            let ia = Index::new(a.clone());
            let ib = Index::new(b.clone());
            let bset: std::collections::HashSet<IndexLabel> = b.iter().cloned().collect();

            let mut seen = std::collections::HashSet::new();
            let ref_inter: Vec<IndexLabel> = a
                .iter()
                .filter(|l| bset.contains(*l) && seen.insert((*l).clone()))
                .cloned()
                .collect();
            assert_eq!(
                ia.intersection(&ib).labels(),
                ref_inter.as_slice(),
                "intersection {a:?} ∩ {b:?}"
            );

            let mut seen_d = std::collections::HashSet::new();
            let ref_diff: Vec<IndexLabel> = a
                .iter()
                .filter(|l| !bset.contains(*l) && seen_d.insert((*l).clone()))
                .cloned()
                .collect();
            assert_eq!(
                ia.difference(&ib).labels(),
                ref_diff.as_slice(),
                "difference {a:?} \\ {b:?}"
            );
        }
    }

    #[test]
    fn datetime_timedelta_get_loc_binary_search_matches_linear_idxdup() {
        // get_loc now binary-searches a monotonic typed index; the result must
        // equal a linear first-match reference for both sorted (binary path) and
        // unsorted (linear path) value vectors.
        for nanos in [
            vec![10_i64, 20, 30, 40, 50], // sorted -> AscendingDatetime64/Timedelta64
            vec![30_i64, 10, 50, 20, 40], // unsorted -> linear fallback
        ] {
            let dt = DatetimeIndex::new(nanos.clone());
            let td = TimedeltaIndex::new(nanos.clone());
            for q in [10_i64, 20, 30, 40, 50, 0, 99] {
                let expected = nanos.iter().position(|n| *n == q);
                assert_eq!(
                    dt.get_loc(q).ok(),
                    expected,
                    "datetime nanos={nanos:?} q={q}"
                );
                assert_eq!(
                    td.get_loc(q).ok(),
                    expected,
                    "timedelta nanos={nanos:?} q={q}"
                );
            }
        }
    }

    #[test]
    fn get_indexer_sorted_fast_path_matches_reference_idxdup() {
        // get_indexer's sorted merge / binary-search fast paths and the
        // FxHashMap fallback must all equal a first-occurrence reference.
        let s = |v: &[i64]| v.iter().map(|x| IndexLabel::Int64(*x)).collect::<Vec<_>>();
        let cases: Vec<(Vec<IndexLabel>, Vec<IndexLabel>)> = vec![
            (s(&[1, 2, 3, 4, 5]), s(&[2, 4, 6])),    // both sorted
            (s(&[1, 2, 3, 4, 5]), s(&[5, 1, 3, 9])), // self sorted, target unsorted
            (s(&[3, 1, 5, 2]), s(&[1, 2, 3])),       // self unsorted -> hash path
            (s(&[1, 2, 3]), vec![]),                 // empty target
            (vec![], s(&[1, 2])),                    // empty self
            (
                vec!["a".into(), "c".into(), "e".into()],
                vec!["c".into(), "z".into(), "a".into()],
            ), // utf8, target unsorted
            (s(&[1, 2, 3]), vec!["a".into()]),       // mixed type sorted, no match
        ];
        for (a, b) in cases {
            let ia = Index::new(a.clone());
            let ib = Index::new(b.clone());
            let ref_out: Vec<Option<usize>> =
                b.iter().map(|t| a.iter().position(|x| x == t)).collect();
            assert_eq!(ia.get_indexer(&ib), ref_out, "get_indexer {a:?} -> {b:?}");
        }
    }

    #[test]
    fn known_unique_constructor_seeds_duplicate_cache() {
        let index = Index::new_known_unique(vec![1_i64.into(), 2_i64.into(), 3_i64.into()]);
        assert_eq!(index.duplicate_cache.get(), Some(&false));
        assert!(!index.has_duplicates());
    }

    #[test]
    fn index_equality_ignores_duplicate_cache_state() {
        let index_with_cache = Index::new(vec!["a".into(), "a".into(), "b".into()]);
        assert!(index_with_cache.has_duplicates());

        let fresh_index = Index::new(vec!["a".into(), "a".into(), "b".into()]);
        assert_eq!(index_with_cache, fresh_index);
    }

    #[test]
    fn index_label_identity_cache_preserves_equality_contracts() {
        let base = Index::new(vec![1_i64.into(), 2_i64.into(), 3_i64.into()]);
        let clone = base.clone();
        assert_eq!(base.label_identity, clone.label_identity);
        assert_eq!(base, clone);

        let renamed = clone.rename_index(Some("rows"));
        assert_eq!(base.label_identity, renamed.label_identity);
        assert!(base.equals(&renamed));
        assert!(!base.identical(&renamed));

        let independent_equal = Index::new(vec![1_i64.into(), 2_i64.into(), 3_i64.into()]);
        assert_ne!(base.label_identity, independent_equal.label_identity);
        assert_eq!(base, independent_equal);

        let different = Index::new(vec![1_i64.into(), 2_i64.into(), 4_i64.into()]);
        assert_ne!(base, different);
        assert!(!base.equals(&different));
    }

    #[test]
    fn semantic_fingerprint_cache_reuses_label_result() {
        let index = Index::new(vec![1_i64.into(), 2_i64.into(), 3_i64.into()]);
        let calls = std::cell::Cell::new(0);

        let first = index.semantic_labels_fingerprint_with(|labels| {
            calls.set(calls.get() + 1);
            format!("labels:{}", labels.len())
        });
        let second = index.semantic_labels_fingerprint_with(|_| {
            calls.set(calls.get() + 1);
            "changed".to_owned()
        });

        assert_eq!(first, "labels:3");
        assert_eq!(second, "labels:3");
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn int64_unit_range_index_preserves_materialized_surface() {
        let index = Index::new_known_unique_int64_unit_range(-2, 4).rename_index(Some("idx"));
        let reference = Index::new(vec![
            IndexLabel::Int64(-2),
            IndexLabel::Int64(-1),
            IndexLabel::Int64(0),
            IndexLabel::Int64(1),
        ])
        .rename_index(Some("idx"));

        assert_eq!(index.len(), 4);
        assert!(!index.has_duplicates());
        assert!(index.is_sorted());
        assert_eq!(index.position(&IndexLabel::Int64(0)), Some(2));
        assert_eq!(index.position(&IndexLabel::Int64(2)), None);
        assert_eq!(index.labels(), reference.labels());
        assert_eq!(index, reference);
    }

    #[test]
    fn int64_affine_range_index_preserves_materialized_surface() {
        let index = Index::new_known_unique_int64_affine_range(3, 2, 4)
            .unwrap()
            .rename_index(Some("idx"));
        let reference = Index::new(vec![
            IndexLabel::Int64(3),
            IndexLabel::Int64(5),
            IndexLabel::Int64(7),
            IndexLabel::Int64(9),
        ])
        .rename_index(Some("idx"));

        assert_eq!(index.len(), 4);
        assert!(!index.has_duplicates());
        assert!(index.is_sorted());
        assert_eq!(index.position(&IndexLabel::Int64(7)), Some(2));
        assert_eq!(index.position(&IndexLabel::Int64(8)), None);
        assert_eq!(index.labels(), reference.labels());
        let values = index.int64_label_values().unwrap();
        assert_eq!(values.as_slice(), &[3, 5, 7, 9]);
        assert_eq!(index, reference);

        let singleton = Index::new_known_unique_int64_affine_range(i64::MAX, 1, 1).unwrap();
        assert_eq!(singleton.labels(), &[IndexLabel::Int64(i64::MAX)]);
    }

    #[test]
    fn int64_strided_index_preserves_duplicate_and_unsorted_semantics() {
        let duplicate =
            Index::from_i64_strided_values(Arc::new(vec![10, 99, 10, 99, 20]), 0, 2, 3).unwrap();
        assert_eq!(
            duplicate.labels(),
            &[
                IndexLabel::Int64(10),
                IndexLabel::Int64(10),
                IndexLabel::Int64(20),
            ]
        );
        assert!(duplicate.has_duplicates());
        assert!(!duplicate.is_sorted());
        assert_eq!(duplicate.position(&IndexLabel::Int64(20)), Some(2));

        let unsorted =
            Index::from_i64_strided_values(Arc::new(vec![30, 99, 10, 99, 20]), 0, 2, 3).unwrap();
        assert_eq!(int64_labels(&unsorted), vec![30, 10, 20]);
        assert!(!unsorted.has_duplicates());
        assert!(!unsorted.is_sorted());
        assert_eq!(unsorted.position(&IndexLabel::Int64(10)), Some(1));
    }

    #[test]
    fn index_variant_wrappers_expose_public_type_surface() {
        let range = RangeIndex::new(1, 7, 2).unwrap().set_name("row");
        assert_eq!(range.values(), vec![1, 3, 5]);
        assert_eq!(range.to_list(), range.values());
        assert_eq!(range.tolist(), range.values());
        assert_eq!(range.to_numpy(), range.values());
        assert_eq!(range.array(), range.values());
        assert_eq!(range.len(), 3);
        assert_eq!(range.size(), 3);
        assert_eq!(range.shape(), (3,));
        assert!(!range.empty());
        assert_eq!(range.dtype(), "int64");
        assert_eq!(range.dtypes(), vec!["int64"]);
        assert_eq!(range.names(), vec![Some("row".to_owned())]);
        assert_eq!(range.copy(), range);
        assert_eq!(range.rename_index(None).name(), None);
        assert_eq!(range.nbytes(), 3 * std::mem::size_of::<i64>());
        assert_eq!(range.to_index().name(), Some("row"));
        assert!(RangeIndex::new(0, 5, 0).is_err());

        let dt = DatetimeIndex::new(vec![1_706_918_400_000_000_000, i64::MIN]).set_name("when");
        assert_eq!(dt.year(), vec![Some(2024), None]);
        assert_eq!(dt.month(), vec![Some(2), None]);
        assert_eq!(dt.day(), vec![Some(3), None]);
        assert_eq!(dt.values(), vec![Some(1_706_918_400_000_000_000), None]);
        assert_eq!(dt.to_list(), dt.values());
        assert_eq!(dt.tolist(), dt.values());
        assert_eq!(dt.to_numpy(), dt.values());
        assert_eq!(dt.array(), dt.values());
        assert_eq!(dt.size(), 2);
        assert_eq!(dt.shape(), (2,));
        assert!(!dt.empty());
        assert_eq!(dt.dtype(), "datetime64[ns]");
        assert_eq!(dt.dtypes(), vec!["datetime64[ns]"]);
        assert_eq!(dt.names(), vec![Some("when".to_owned())]);
        assert_eq!(dt.copy(), dt);
        assert!(dt.hasnans());
        assert_eq!(dt.isna(), vec![false, true]);
        assert_eq!(dt.notna(), vec![true, false]);
        assert!(dt.nbytes() <= dt.memory_usage(true));
        assert!(DatetimeIndex::from_index(Index::from_i64(vec![1])).is_err());

        let td = TimedeltaIndex::new(vec![90_061_000_000_000, Timedelta::NAT]).set_name("delta");
        assert_eq!(td.days(), vec![Some(1), None]);
        assert_eq!(td.seconds(), vec![Some(3661), None]);
        assert_eq!(td.total_seconds(), vec![Some(90061.0), None]);
        assert_eq!(td.values(), vec![Some(90_061_000_000_000), None]);
        assert_eq!(td.to_list(), td.values());
        assert_eq!(td.tolist(), td.values());
        assert_eq!(td.to_numpy(), td.values());
        assert_eq!(td.array(), td.values());
        assert_eq!(td.size(), 2);
        assert_eq!(td.shape(), (2,));
        assert!(!td.empty());
        assert_eq!(td.dtype(), "timedelta64[ns]");
        assert_eq!(td.dtypes(), vec!["timedelta64[ns]"]);
        assert_eq!(td.names(), vec![Some("delta".to_owned())]);
        assert_eq!(td.copy(), td);
        assert!(td.hasnans());
        assert_eq!(td.isna(), vec![false, true]);
        assert_eq!(td.notna(), vec![true, false]);

        let period =
            PeriodIndex::from_range(Period::new(10, PeriodFreq::Monthly), 3).set_name("period");
        assert_eq!(period.freq(), Some(PeriodFreq::Monthly));
        assert_eq!(
            period.values(),
            &[
                Period::new(10, PeriodFreq::Monthly),
                Period::new(11, PeriodFreq::Monthly),
                Period::new(12, PeriodFreq::Monthly),
            ]
        );
        assert_eq!(period.to_list(), period.values());
        assert_eq!(period.tolist(), period.values());
        assert_eq!(period.to_numpy(), period.values());
        assert_eq!(period.array(), period.values());
        assert_eq!(period.size(), 3);
        assert_eq!(period.shape(), (3,));
        assert!(!period.empty());
        assert_eq!(period.dtype(), "period[M]");
        assert_eq!(period.dtypes(), vec!["period[M]".to_owned()]);
        assert_eq!(period.names(), vec![Some("period".to_owned())]);
        assert_eq!(period.copy(), period);
        assert_eq!(period.rename_index(None).name(), None);
        assert!(period.nbytes() <= period.memory_usage(true));
        assert_eq!(period.to_index().name(), Some("period"));

        let categorical = CategoricalIndex::from_values(
            vec!["low".to_owned(), "high".to_owned(), "low".to_owned()],
            true,
        )
        .set_name("priority");
        assert_eq!(categorical.categories(), &["low", "high"]);
        assert_eq!(categorical.codes(), vec![Some(0), Some(1), Some(0)]);
        assert!(categorical.ordered());
        assert_eq!(
            categorical.values(),
            vec!["low".to_owned(), "high".to_owned(), "low".to_owned()]
        );
        assert_eq!(categorical.to_list(), categorical.values());
        assert_eq!(categorical.tolist(), categorical.values());
        assert_eq!(categorical.to_numpy(), categorical.values());
        assert_eq!(categorical.array(), categorical.values());
        assert_eq!(categorical.size(), 3);
        assert_eq!(categorical.shape(), (3,));
        assert!(!categorical.empty());
        assert_eq!(categorical.dtype(), "category");
        assert_eq!(categorical.dtypes(), vec!["category"]);
        assert_eq!(categorical.names(), vec![Some("priority".to_owned())]);
        assert_eq!(categorical.copy(), categorical);
        assert_eq!(categorical.isna(), vec![false, false, false]);
        assert_eq!(categorical.notna(), vec![true, true, true]);
        assert!(categorical.nbytes() <= categorical.memory_usage(true));
        assert_eq!(categorical.to_index().name(), Some("priority"));
        assert!(
            CategoricalIndex::with_categories(
                vec!["missing".to_owned()],
                vec!["known".to_owned()],
                false,
            )
            .is_err()
        );
    }

    #[test]
    fn typed_index_str_accessors_forward_flat_labels_e7ms9() -> Result<(), super::IndexError> {
        let flat = Index::new(vec!["Alpha".into(), 1_i64.into(), "".into()]);
        assert_eq!(
            flat.r#str().lower(),
            vec![Some("alpha".to_owned()), None, Some(String::new())]
        );

        let range = RangeIndex::new(1, 4, 1)?;
        assert_eq!(range.r#str().len(), vec![None, None, None]);

        let dt = DatetimeIndex::new(vec![1_704_067_200_000_000_000]);
        assert_eq!(dt.r#str().upper(), vec![None]);

        let td = TimedeltaIndex::new(vec![90_061_000_000_000]);
        assert_eq!(td.r#str().contains("day"), vec![None]);

        let period = PeriodIndex::from_range(Period::new(10, PeriodFreq::Monthly), 2);
        let expected_period_lower: Vec<Option<String>> = period
            .format()
            .into_iter()
            .map(|label| Some(label.to_lowercase()))
            .collect();
        assert_eq!(period.r#str().lower(), expected_period_lower);

        let categorical = CategoricalIndex::from_values(
            vec!["Low".to_owned(), "HIGH".to_owned(), String::new()],
            false,
        );
        assert_eq!(
            categorical.r#str().lower(),
            vec![
                Some("low".to_owned()),
                Some("high".to_owned()),
                Some(String::new())
            ]
        );
        Ok(())
    }

    #[test]
    fn period_index_from_fields_builds_period_ordinals_th1fd() -> Result<(), super::IndexError> {
        let years = [2020, 2021];
        let months = [1, 2];
        let monthly = PeriodIndex::from_fields(PeriodFields {
            month: Some(&months),
            freq: Some(PeriodFreq::Monthly),
            ..PeriodFields::new(&years)
        })?;
        assert_eq!(
            monthly.values(),
            &[
                Period::new(600, PeriodFreq::Monthly),
                Period::new(613, PeriodFreq::Monthly)
            ]
        );

        let quarter_years = [2020];
        let quarters = [2];
        let quarterly = PeriodIndex::from_fields(PeriodFields {
            quarter: Some(&quarters),
            ..PeriodFields::new(&quarter_years)
        })?;
        assert_eq!(
            quarterly.values(),
            &[Period::new(201, PeriodFreq::Quarterly)]
        );

        let single_year = [2020];
        let single_month = [1];
        let weekly = PeriodIndex::from_fields(PeriodFields {
            month: Some(&single_month),
            freq: Some(PeriodFreq::Weekly),
            ..PeriodFields::new(&single_year)
        })?;
        assert_eq!(weekly.values(), &[Period::new(2_610, PeriodFreq::Weekly)]);

        let weekend_day = [4];
        let business = PeriodIndex::from_fields(PeriodFields {
            month: Some(&single_month),
            day: Some(&weekend_day),
            freq: Some(PeriodFreq::Business),
            ..PeriodFields::new(&single_year)
        })?;
        assert_eq!(
            business.values(),
            &[Period::new(13_047, PeriodFreq::Business)]
        );

        let days = [2];
        let hours = [3];
        let minutes = [4];
        let seconds = [5];
        let secondly = PeriodIndex::from_fields(PeriodFields {
            month: Some(&single_month),
            day: Some(&days),
            hour: Some(&hours),
            minute: Some(&minutes),
            second: Some(&seconds),
            freq: Some(PeriodFreq::Secondly),
            ..PeriodFields::new(&single_year)
        })?;
        let expected_date = chrono::NaiveDate::from_ymd_opt(2020, 1, 2)
            .ok_or_else(|| super::IndexError::InvalidArgument("invalid test date".to_owned()))?;
        let expected_time = chrono::NaiveTime::from_hms_opt(3, 4, 5)
            .ok_or_else(|| super::IndexError::InvalidArgument("invalid test time".to_owned()))?;
        let expected_nanos =
            super::date_and_time_to_nanos(expected_date, super::time_to_nanos(expected_time))
                .map_err(super::period_date_error)?;
        assert_eq!(
            secondly.values(),
            &[Period::new(
                super::datetime_period_ordinal(expected_nanos, PeriodFreq::Secondly)?,
                PeriodFreq::Secondly
            )]
        );

        assert!(
            PeriodIndex::from_fields(PeriodFields {
                month: Some(&months),
                freq: Some(PeriodFreq::Monthly),
                ..PeriodFields::new(&single_year)
            })
            .is_err()
        );
        let invalid_month = [13];
        assert!(
            PeriodIndex::from_fields(PeriodFields {
                month: Some(&invalid_month),
                freq: Some(PeriodFreq::Monthly),
                ..PeriodFields::new(&single_year)
            })
            .is_err()
        );
        let invalid_day = [99];
        assert_eq!(
            PeriodIndex::from_fields(PeriodFields {
                month: Some(&single_month),
                day: Some(&invalid_day),
                freq: Some(PeriodFreq::Monthly),
                ..PeriodFields::new(&single_year)
            })?
            .values(),
            &[Period::new(600, PeriodFreq::Monthly)]
        );
        assert!(
            PeriodIndex::from_fields(PeriodFields {
                quarter: Some(&quarters),
                month: Some(&single_month),
                freq: Some(PeriodFreq::Monthly),
                ..PeriodFields::new(&single_year)
            })
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn index_variant_wrappers_expose_identity_and_type_surface() {
        let range = RangeIndex::new(1, 7, 2).unwrap().set_name("row");
        assert!(range.is_(&range));
        assert!(range.equals(&range.copy()));
        assert!(range.identical(&range.copy()));
        assert!(!range.identical(&range.rename_index(None)));
        assert!(range.is_unique());
        assert!(!range.has_duplicates());
        assert!(range.is_monotonic_increasing());
        assert!(!range.is_monotonic_decreasing());
        assert_eq!(range.nunique(), 3);
        assert_eq!(range.ndim(), 1);
        assert_eq!(RangeIndex::new(4, 5, 1).unwrap().item().unwrap(), 4);
        assert!(range.item().is_err());
        assert!(range.holds_integer());
        assert_eq!(range.inferred_type(), "integer");
        assert!(range.is_integer());
        assert!(range.is_numeric());
        assert!(!range.is_boolean());
        assert!(!range.is_categorical());
        assert!(!range.is_floating());
        assert!(!range.is_interval());
        assert!(!range.is_object());

        let dt = DatetimeIndex::new(vec![1_706_918_400_000_000_000, i64::MIN]).set_name("when");
        assert!(dt.is_(&dt));
        assert!(dt.equals(&dt.copy()));
        assert!(dt.identical(&dt.copy()));
        assert!(!dt.identical(&dt.rename_index(None)));
        assert!(dt.is_unique());
        assert!(!dt.has_duplicates());
        assert_eq!(dt.nunique(), 1);
        assert_eq!(dt.nunique_with_dropna(false), 2);
        assert_eq!(dt.ndim(), 1);
        assert_eq!(
            DatetimeIndex::new(vec![1_706_918_400_000_000_000])
                .item()
                .unwrap(),
            Some(1_706_918_400_000_000_000)
        );
        assert_eq!(DatetimeIndex::new(vec![i64::MIN]).item().unwrap(), None);
        assert_eq!(dt.inferred_type(), "datetime64");
        assert!(!dt.holds_integer());
        assert!(!dt.is_integer());
        assert!(!dt.is_numeric());
        assert!(!dt.is_boolean());
        assert!(!dt.is_categorical());
        assert!(!dt.is_floating());
        assert!(!dt.is_interval());
        assert!(!dt.is_object());
        assert!(DatetimeIndex::new(vec![1, 2]).is_monotonic_increasing());
        assert!(DatetimeIndex::new(vec![2, 1]).is_monotonic_decreasing());

        let td = TimedeltaIndex::new(vec![1, Timedelta::NAT]).set_name("delta");
        assert!(td.is_(&td));
        assert!(td.equals(&td.copy()));
        assert!(td.identical(&td.copy()));
        assert!(!td.identical(&td.rename_index(None)));
        assert!(td.is_unique());
        assert_eq!(td.nunique(), 1);
        assert_eq!(td.nunique_with_dropna(false), 2);
        assert_eq!(td.ndim(), 1);
        assert_eq!(TimedeltaIndex::new(vec![7]).item().unwrap(), Some(7));
        assert_eq!(
            TimedeltaIndex::new(vec![Timedelta::NAT]).item().unwrap(),
            None
        );
        assert_eq!(td.inferred_type(), "timedelta64");
        assert!(!td.holds_integer());
        assert!(!td.is_integer());
        assert!(!td.is_numeric());
        assert!(!td.is_boolean());
        assert!(!td.is_categorical());
        assert!(!td.is_floating());
        assert!(!td.is_interval());
        assert!(!td.is_object());
        assert!(TimedeltaIndex::new(vec![1, 2]).is_monotonic_increasing());
        assert!(TimedeltaIndex::new(vec![2, 1]).is_monotonic_decreasing());

        let period =
            PeriodIndex::from_range(Period::new(10, PeriodFreq::Monthly), 3).set_name("period");
        assert!(period.is_(&period));
        assert!(period.equals(&period.copy()));
        assert!(period.identical(&period.copy()));
        assert!(!period.identical(&period.rename_index(None)));
        assert!(period.is_unique());
        assert!(!period.has_duplicates());
        assert!(period.is_monotonic_increasing());
        assert!(!period.is_monotonic_decreasing());
        assert_eq!(period.nunique(), 3);
        assert_eq!(period.ndim(), 1);
        assert_eq!(
            PeriodIndex::new(vec![Period::new(42, PeriodFreq::Daily)])
                .item()
                .unwrap(),
            Period::new(42, PeriodFreq::Daily)
        );
        assert_eq!(period.inferred_type(), "period");
        assert!(!period.holds_integer());
        assert!(!period.is_integer());
        assert!(!period.is_numeric());
        assert!(!period.is_boolean());
        assert!(!period.is_categorical());
        assert!(!period.is_floating());
        assert!(!period.is_interval());
        assert!(!period.is_object());

        let categorical = CategoricalIndex::from_values(
            vec!["low".to_owned(), "high".to_owned(), "low".to_owned()],
            true,
        )
        .set_name("priority");
        assert!(categorical.is_(&categorical));
        assert!(categorical.equals(&categorical.copy()));
        assert!(categorical.identical(&categorical.copy()));
        assert!(!categorical.identical(&categorical.rename_index(None)));
        assert!(!categorical.is_unique());
        assert!(categorical.has_duplicates());
        assert_eq!(categorical.nunique(), 2);
        assert_eq!(categorical.ndim(), 1);
        assert_eq!(
            CategoricalIndex::from_values(vec!["high".to_owned()], true)
                .item()
                .unwrap(),
            "high"
        );
        assert_eq!(categorical.inferred_type(), "categorical");
        assert!(!categorical.holds_integer());
        assert!(!categorical.is_integer());
        assert!(!categorical.is_numeric());
        assert!(!categorical.is_boolean());
        assert!(categorical.is_categorical());
        assert!(!categorical.is_floating());
        assert!(!categorical.is_interval());
        assert!(!categorical.is_object());
        assert!(!categorical.is_monotonic_increasing());
        assert!(!categorical.is_monotonic_decreasing());
        assert!(
            CategoricalIndex::from_values(vec!["low".to_owned(), "high".to_owned()], true)
                .is_monotonic_increasing()
        );
    }

    // === AG-13: Adaptive Index Backend Tests ===

    #[test]
    fn sorted_int64_index_detected() {
        let index = Index::from_i64(vec![1, 2, 3, 4, 5]);
        assert!(index.is_sorted());
    }

    #[test]
    fn unsorted_int64_index_detected() {
        let index = Index::from_i64(vec![3, 1, 2]);
        assert!(!index.is_sorted());
    }

    #[test]
    fn sorted_utf8_index_detected() {
        let index = Index::from_utf8(vec!["a".into(), "b".into(), "c".into()]);
        assert!(index.is_sorted());
    }

    #[test]
    fn unsorted_utf8_index_detected() {
        let index = Index::from_utf8(vec!["c".into(), "a".into(), "b".into()]);
        assert!(!index.is_sorted());
    }

    #[test]
    fn duplicate_int64_is_not_sorted() {
        let index = Index::from_i64(vec![1, 2, 2, 3]);
        assert!(!index.is_sorted());
    }

    #[test]
    fn empty_index_is_sorted() {
        let index = Index::new(vec![]);
        assert!(index.is_sorted());
    }

    #[test]
    fn single_element_is_sorted() {
        let index = Index::from_i64(vec![42]);
        assert!(index.is_sorted());
    }

    #[test]
    fn binary_search_position_sorted_int64() {
        let index = Index::from_i64(vec![10, 20, 30, 40, 50]);
        assert_eq!(index.position(&IndexLabel::Int64(10)), Some(0));
        assert_eq!(index.position(&IndexLabel::Int64(30)), Some(2));
        assert_eq!(index.position(&IndexLabel::Int64(50)), Some(4));
        assert_eq!(index.position(&IndexLabel::Int64(25)), None);
        assert_eq!(index.position(&IndexLabel::Int64(0)), None);
        assert_eq!(index.position(&IndexLabel::Int64(100)), None);
    }

    #[test]
    fn binary_search_position_sorted_utf8() {
        let index = Index::from_utf8(vec!["apple".into(), "banana".into(), "cherry".into()]);
        assert_eq!(index.position(&IndexLabel::Utf8("apple".into())), Some(0));
        assert_eq!(index.position(&IndexLabel::Utf8("banana".into())), Some(1));
        assert_eq!(index.position(&IndexLabel::Utf8("cherry".into())), Some(2));
        assert_eq!(index.position(&IndexLabel::Utf8("date".into())), None);
    }

    #[test]
    fn type_mismatch_returns_none() {
        let int_index = Index::from_i64(vec![1, 2, 3]);
        // Looking for a Utf8 needle in an Int64 index
        assert_eq!(int_index.position(&IndexLabel::Utf8("1".into())), None);

        let utf8_index = Index::from_utf8(vec!["a".into(), "b".into()]);
        // Looking for an Int64 needle in a Utf8 index
        assert_eq!(utf8_index.position(&IndexLabel::Int64(1)), None);
    }

    #[test]
    fn linear_fallback_for_unsorted_index() {
        let index = Index::from_i64(vec![30, 10, 20]);
        assert!(!index.is_sorted());
        assert_eq!(index.position(&IndexLabel::Int64(30)), Some(0));
        assert_eq!(index.position(&IndexLabel::Int64(10)), Some(1));
        assert_eq!(index.position(&IndexLabel::Int64(20)), Some(2));
        assert_eq!(index.position(&IndexLabel::Int64(99)), None);
    }

    #[test]
    fn binary_search_large_sorted_index() {
        // Verify binary search works correctly on a large sorted index.
        let labels: Vec<i64> = (0..10_000).collect();
        let index = Index::from_i64(labels);
        assert!(index.is_sorted());

        // Check first, middle, last, and missing positions.
        assert_eq!(index.position(&IndexLabel::Int64(0)), Some(0));
        assert_eq!(index.position(&IndexLabel::Int64(5000)), Some(5000));
        assert_eq!(index.position(&IndexLabel::Int64(9999)), Some(9999));
        assert_eq!(index.position(&IndexLabel::Int64(10_000)), None);
        assert_eq!(index.position(&IndexLabel::Int64(-1)), None);
    }

    #[test]
    fn sort_detection_is_cached() {
        let index = Index::from_i64(vec![1, 2, 3]);
        // First call computes and caches.
        assert!(index.is_sorted());
        // Second call should return same result from cache.
        assert!(index.is_sorted());
    }

    #[test]
    fn mixed_label_types_are_unsorted() {
        let index = Index::new(vec![IndexLabel::Int64(1), IndexLabel::Utf8("a".into())]);
        assert!(!index.is_sorted());
    }

    #[test]
    fn position_consistent_sorted_vs_unsorted() {
        // Verify that for a sorted index, binary search gives the same
        // results as a linear scan would.
        let sorted = Index::from_i64(vec![5, 10, 15, 20, 25]);
        assert!(sorted.is_sorted());

        for &target in &[5, 10, 15, 20, 25, 0, 12, 30] {
            let needle = IndexLabel::Int64(target);
            let expected = sorted.labels().iter().position(|l| l == &needle);
            assert_eq!(
                sorted.position(&needle),
                expected,
                "mismatch for target={target}"
            );
        }
    }

    // === bd-2gi.15: Alignment mode tests ===

    use super::{AlignMode, align, align_inner, align_left};

    #[test]
    fn align_inner_keeps_only_overlapping_labels() {
        let left = Index::new(vec![1_i64.into(), 2_i64.into(), 3_i64.into()]);
        let right = Index::new(vec![2_i64.into(), 3_i64.into(), 4_i64.into()]);

        let plan = align_inner(&left, &right);
        assert_eq!(
            plan.union_index.labels(),
            &[IndexLabel::Int64(2), IndexLabel::Int64(3)]
        );
        assert_eq!(plan.left_positions, vec![Some(1), Some(2)]);
        assert_eq!(plan.right_positions, vec![Some(0), Some(1)]);
        validate_alignment_plan(&plan).expect("valid");
    }

    #[test]
    fn align_inner_disjoint_yields_empty() {
        let left = Index::new(vec![1_i64.into(), 2_i64.into()]);
        let right = Index::new(vec![3_i64.into(), 4_i64.into()]);

        let plan = align_inner(&left, &right);
        assert!(plan.union_index.is_empty());
        assert!(plan.left_positions.is_empty());
        assert!(plan.right_positions.is_empty());
    }

    #[test]
    fn align_left_preserves_all_left_labels() {
        let left = Index::new(vec!["a".into(), "b".into(), "c".into()]);
        let right = Index::new(vec!["b".into(), "d".into()]);

        let plan = align_left(&left, &right);
        assert_eq!(
            plan.union_index.labels(),
            &["a".into(), "b".into(), "c".into()]
        );
        assert_eq!(plan.left_positions, vec![Some(0), Some(1), Some(2)]);
        assert_eq!(plan.right_positions, vec![None, Some(0), None]);
        validate_alignment_plan(&plan).expect("valid");
    }

    #[test]
    fn align_right_preserves_all_right_labels() {
        let left = Index::new(vec!["a".into(), "b".into()]);
        let right = Index::new(vec!["b".into(), "c".into(), "d".into()]);

        let plan = align(&left, &right, AlignMode::Right);
        assert_eq!(
            plan.union_index.labels(),
            &["b".into(), "c".into(), "d".into()]
        );
        // Left has "b" at position 1.
        assert_eq!(plan.left_positions, vec![Some(1), None, None]);
        assert_eq!(plan.right_positions, vec![Some(0), Some(1), Some(2)]);
    }

    #[test]
    fn align_mode_outer_matches_union() {
        let left = Index::new(vec![1_i64.into(), 2_i64.into()]);
        let right = Index::new(vec![2_i64.into(), 3_i64.into()]);

        let plan_outer = align(&left, &right, AlignMode::Outer);
        let plan_union = align_union(&left, &right);
        assert_eq!(plan_outer, plan_union);
    }

    #[test]
    fn align_inner_duplicate_labels_cartesian() {
        let left = Index::new(vec!["a".into(), "b".into(), "a".into()]);
        let right = Index::new(vec!["a".into(), "a".into(), "c".into()]);

        let plan = align_inner(&left, &right);
        assert_eq!(
            plan.union_index.labels(),
            &["a".into(), "a".into(), "a".into(), "a".into()]
        );
        assert_eq!(
            plan.left_positions,
            vec![Some(0), Some(0), Some(2), Some(2)]
        );
        assert_eq!(
            plan.right_positions,
            vec![Some(0), Some(1), Some(0), Some(1)]
        );
        validate_alignment_plan(&plan).expect("valid");
    }

    #[test]
    fn align_left_duplicate_labels_expand_right_matches() {
        let left = Index::new(vec!["a".into(), "b".into(), "a".into()]);
        let right = Index::new(vec!["a".into(), "a".into(), "c".into()]);

        let plan = align_left(&left, &right);
        assert_eq!(
            plan.union_index.labels(),
            &["a".into(), "a".into(), "b".into(), "a".into(), "a".into()]
        );
        assert_eq!(
            plan.left_positions,
            vec![Some(0), Some(0), Some(1), Some(2), Some(2)]
        );
        assert_eq!(
            plan.right_positions,
            vec![Some(0), Some(1), None, Some(0), Some(1)]
        );
        validate_alignment_plan(&plan).expect("valid");
    }

    #[test]
    fn align_right_duplicate_labels_expand_left_matches() {
        let left = Index::new(vec!["a".into(), "b".into(), "a".into()]);
        let right = Index::new(vec!["a".into(), "a".into(), "c".into()]);

        let plan = align(&left, &right, AlignMode::Right);
        assert_eq!(
            plan.union_index.labels(),
            &["a".into(), "a".into(), "a".into(), "a".into(), "c".into()]
        );
        assert_eq!(
            plan.left_positions,
            vec![Some(0), Some(2), Some(0), Some(2), None]
        );
        assert_eq!(
            plan.right_positions,
            vec![Some(0), Some(0), Some(1), Some(1), Some(2)]
        );
        validate_alignment_plan(&plan).expect("valid");
    }

    #[test]
    fn align_outer_duplicate_labels_preserves_left_order_and_right_only() {
        let left = Index::new(vec!["a".into(), "b".into(), "a".into()]);
        let right = Index::new(vec!["a".into(), "a".into(), "c".into()]);

        let plan = align_union(&left, &right);
        assert_eq!(
            plan.union_index.labels(),
            &[
                "a".into(),
                "a".into(),
                "b".into(),
                "a".into(),
                "a".into(),
                "c".into()
            ]
        );
        assert_eq!(
            plan.left_positions,
            vec![Some(0), Some(0), Some(1), Some(2), Some(2), None]
        );
        assert_eq!(
            plan.right_positions,
            vec![Some(0), Some(1), None, Some(0), Some(1), Some(2)]
        );
        validate_alignment_plan(&plan).expect("valid");
    }

    #[test]
    fn align_inner_identical_indexes() {
        let left = Index::new(vec!["x".into(), "y".into()]);
        let right = Index::new(vec!["x".into(), "y".into()]);

        let plan = align_inner(&left, &right);
        assert_eq!(plan.union_index.labels(), &["x".into(), "y".into()]);
        assert_eq!(plan.left_positions, vec![Some(0), Some(1)]);
        assert_eq!(plan.right_positions, vec![Some(0), Some(1)]);
    }

    #[test]
    fn align_left_identical_indexes() {
        let left = Index::new(vec![1_i64.into(), 2_i64.into()]);
        let right = Index::new(vec![1_i64.into(), 2_i64.into()]);

        let plan = align_left(&left, &right);
        assert_eq!(plan.union_index.labels(), left.labels());
        assert_eq!(plan.left_positions, vec![Some(0), Some(1)]);
        assert_eq!(plan.right_positions, vec![Some(0), Some(1)]);
    }

    #[test]
    fn align_inner_empty_input() {
        let left = Index::new(Vec::new());
        let right = Index::new(vec![1_i64.into()]);

        let plan = align_inner(&left, &right);
        assert!(plan.union_index.is_empty());
    }

    #[test]
    fn align_left_empty_left() {
        let left = Index::new(Vec::new());
        let right = Index::new(vec![1_i64.into()]);

        let plan = align_left(&left, &right);
        assert!(plan.union_index.is_empty());
    }

    // === bd-2gi.13: Index model and indexer semantics ===

    use super::DuplicateKeep;

    #[test]
    fn contains_finds_existing_label() {
        let index = Index::from_i64(vec![10, 20, 30]);
        assert!(index.contains(&IndexLabel::Int64(20)));
        assert!(!index.contains(&IndexLabel::Int64(99)));
    }

    #[test]
    fn get_indexer_bulk_lookup() {
        let index = Index::new(vec!["a".into(), "b".into(), "c".into()]);
        let target = Index::new(vec!["c".into(), "a".into(), "z".into()]);
        assert_eq!(index.get_indexer(&target), vec![Some(2), Some(0), None]);
    }

    #[test]
    fn isin_membership_mask() {
        let index = Index::from_i64(vec![1, 2, 3, 4, 5]);
        let values = vec![IndexLabel::Int64(2), IndexLabel::Int64(4)];
        assert_eq!(index.isin(&values), vec![false, true, false, true, false]);
    }

    #[test]
    fn unique_preserves_first_seen_order() {
        let index = Index::new(vec![
            "b".into(),
            "a".into(),
            "b".into(),
            "c".into(),
            "a".into(),
        ]);
        let uniq = index.unique();
        assert_eq!(uniq.labels(), &["b".into(), "a".into(), "c".into()]);
    }

    #[test]
    fn duplicated_keep_first() {
        let index = Index::from_i64(vec![1, 2, 1, 3, 2]);
        assert_eq!(
            index.duplicated(DuplicateKeep::First),
            vec![false, false, true, false, true]
        );
    }

    #[test]
    fn duplicated_keep_last() {
        let index = Index::from_i64(vec![1, 2, 1, 3, 2]);
        assert_eq!(
            index.duplicated(DuplicateKeep::Last),
            vec![true, true, false, false, false]
        );
    }

    #[test]
    fn duplicated_keep_none_marks_all() {
        let index = Index::from_i64(vec![1, 2, 1, 3, 2]);
        assert_eq!(
            index.duplicated(DuplicateKeep::None),
            vec![true, true, true, false, true]
        );
    }

    #[test]
    fn drop_duplicates_equals_unique() {
        let index = Index::from_i64(vec![3, 1, 3, 2, 1]);
        assert_eq!(index.drop_duplicates(), index.unique());
    }

    #[test]
    fn index_drop_duplicates_keep_last() {
        let index = Index::new(vec![
            "llama".into(),
            "cow".into(),
            "llama".into(),
            "beetle".into(),
            "llama".into(),
            "hippo".into(),
        ])
        .set_names(Some("animals"));

        let deduped = index.drop_duplicates_keep(DuplicateKeep::Last);

        assert_eq!(
            deduped.labels(),
            &[
                IndexLabel::from("cow"),
                IndexLabel::from("beetle"),
                IndexLabel::from("llama"),
                IndexLabel::from("hippo"),
            ]
        );
        assert_eq!(deduped.name(), Some("animals"));
    }

    #[test]
    fn index_drop_duplicates_keep_none_discards_all_duplicates() {
        let index = Index::new(vec![
            "llama".into(),
            "cow".into(),
            "llama".into(),
            "beetle".into(),
            "llama".into(),
            "hippo".into(),
        ]);

        let deduped = index.drop_duplicates_keep(DuplicateKeep::None);

        assert_eq!(
            deduped.labels(),
            &[
                IndexLabel::from("cow"),
                IndexLabel::from("beetle"),
                IndexLabel::from("hippo"),
            ]
        );
    }

    #[test]
    fn intersection_preserves_left_order() {
        let left = Index::new(vec!["c".into(), "a".into(), "b".into()]);
        let right = Index::new(vec!["b".into(), "d".into(), "a".into()]);
        let result = left.intersection(&right);
        assert_eq!(result.labels(), &["a".into(), "b".into()]);
    }

    #[test]
    fn intersection_deduplicates() {
        let left = Index::from_i64(vec![1, 1, 2]);
        let right = Index::from_i64(vec![1, 2, 2]);
        let result = left.intersection(&right);
        assert_eq!(
            result.labels(),
            &[IndexLabel::Int64(1), IndexLabel::Int64(2)]
        );
    }

    #[test]
    fn union_with_combines_unique_labels() {
        let left = Index::from_i64(vec![1, 2, 3]);
        let right = Index::from_i64(vec![2, 4, 3]);
        let result = left.union_with(&right);
        assert_eq!(
            result.labels(),
            &[
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
                IndexLabel::Int64(3),
                IndexLabel::Int64(4),
            ]
        );
    }

    #[test]
    fn difference_removes_other_labels() {
        let left = Index::from_i64(vec![1, 2, 3, 4]);
        let right = Index::from_i64(vec![2, 4]);
        let result = left.difference(&right);
        assert_eq!(
            result.labels(),
            &[IndexLabel::Int64(1), IndexLabel::Int64(3)]
        );
    }

    #[test]
    fn difference_preserves_self_name_even_when_other_differs_6r1lq() {
        // Per br-frankenpandas-6r1lq: difference is asymmetric — pandas
        // preserves self.name regardless of whether other has the same name.
        let left = Index::from_i64(vec![1, 2, 3]).set_name("left_axis");
        let right = Index::from_i64(vec![2, 3, 4]).set_name("right_axis");
        let result = left.difference(&right);
        assert_eq!(result.name(), Some("left_axis"));
    }

    #[test]
    fn symmetric_difference_xor() {
        let left = Index::from_i64(vec![1, 2, 3]);
        let right = Index::from_i64(vec![2, 3, 4]);
        let result = left.symmetric_difference(&right);
        assert_eq!(
            result.labels(),
            &[IndexLabel::Int64(1), IndexLabel::Int64(4)]
        );
    }

    #[test]
    fn argsort_returns_sorting_indices() {
        let index = Index::from_i64(vec![30, 10, 20]);
        assert_eq!(index.argsort(), vec![1, 2, 0]);
    }

    #[test]
    fn sort_values_produces_sorted_index() {
        let index = Index::new(vec!["c".into(), "a".into(), "b".into()]);
        let sorted = index.sort_values();
        assert_eq!(sorted.labels(), &["a".into(), "b".into(), "c".into()]);
    }

    #[test]
    fn take_selects_by_position() {
        let index = Index::from_i64(vec![10, 20, 30, 40, 50]);
        let taken = index.take(&[4, 0, 2]);
        assert_eq!(
            taken.labels(),
            &[
                IndexLabel::Int64(50),
                IndexLabel::Int64(10),
                IndexLabel::Int64(30),
            ]
        );
    }

    #[test]
    fn slice_extracts_subrange() {
        let index = Index::from_i64(vec![10, 20, 30, 40, 50]);
        let sliced = index.slice(1, 3);
        assert_eq!(
            sliced.labels(),
            &[
                IndexLabel::Int64(20),
                IndexLabel::Int64(30),
                IndexLabel::Int64(40),
            ]
        );
    }

    #[test]
    fn slice_of_materialized_index_keeps_shared_label_view() {
        let index = Index::new(vec![
            IndexLabel::Utf8("a".into()),
            IndexLabel::Utf8("b".into()),
            IndexLabel::Utf8("c".into()),
            IndexLabel::Utf8("d".into()),
        ]);
        let sliced = index.slice(1, 2);

        assert!(sliced.labels.materialized_slice.is_some());
        assert!(sliced.labels.materialized.get().is_none());
        assert_eq!(
            sliced.labels(),
            &[IndexLabel::Utf8("b".into()), IndexLabel::Utf8("c".into()),]
        );
    }

    #[test]
    fn slice_clamps_to_bounds() {
        let index = Index::from_i64(vec![1, 2, 3]);
        let sliced = index.slice(1, 100);
        assert_eq!(
            sliced.labels(),
            &[IndexLabel::Int64(2), IndexLabel::Int64(3)]
        );
    }

    #[test]
    fn from_range_basic() {
        let index = Index::from_range(0, 5, 1);
        assert_eq!(
            index.labels(),
            &[
                IndexLabel::Int64(0),
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
                IndexLabel::Int64(3),
                IndexLabel::Int64(4),
            ]
        );
    }

    #[test]
    fn from_range_step_2() {
        let index = Index::from_range(0, 10, 3);
        assert_eq!(
            index.labels(),
            &[
                IndexLabel::Int64(0),
                IndexLabel::Int64(3),
                IndexLabel::Int64(6),
                IndexLabel::Int64(9),
            ]
        );
    }

    #[test]
    fn from_range_negative_step() {
        let index = Index::from_range(5, 0, -2);
        assert_eq!(
            index.labels(),
            &[
                IndexLabel::Int64(5),
                IndexLabel::Int64(3),
                IndexLabel::Int64(1),
            ]
        );
    }

    #[test]
    fn from_range_empty_when_step_zero() {
        let index = Index::from_range(0, 5, 0);
        assert!(index.is_empty());
    }

    #[test]
    fn set_ops_empty_inputs() {
        let empty = Index::new(Vec::new());
        let non_empty = Index::from_i64(vec![1, 2]);
        assert!(empty.intersection(&non_empty).is_empty());
        assert_eq!(empty.union_with(&non_empty), non_empty);
        assert!(empty.difference(&non_empty).is_empty());
        assert_eq!(empty.symmetric_difference(&non_empty), non_empty);
    }

    // === AG-11: Leapfrog Triejoin Tests ===

    use super::{leapfrog_intersection, leapfrog_union, multi_way_align};

    #[test]
    fn leapfrog_union_three_indexes() {
        let a = Index::from_i64(vec![1, 3, 5]);
        let b = Index::from_i64(vec![2, 3, 6]);
        let c = Index::from_i64(vec![4, 5, 6]);
        let result = leapfrog_union(&[&a, &b, &c]);
        assert_eq!(
            result.labels(),
            &[
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
                IndexLabel::Int64(3),
                IndexLabel::Int64(4),
                IndexLabel::Int64(5),
                IndexLabel::Int64(6),
            ]
        );
    }

    #[test]
    fn leapfrog_union_deduplicates() {
        let a = Index::from_i64(vec![1, 1, 2]);
        let b = Index::from_i64(vec![2, 2, 3]);
        let result = leapfrog_union(&[&a, &b]);
        assert_eq!(
            result.labels(),
            &[
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
                IndexLabel::Int64(3),
            ]
        );
    }

    #[test]
    fn leapfrog_union_single_index() {
        let a = Index::from_i64(vec![3, 1, 2]);
        let result = leapfrog_union(&[&a]);
        assert_eq!(
            result.labels(),
            &[
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
                IndexLabel::Int64(3),
            ]
        );
    }

    #[test]
    fn leapfrog_union_empty() {
        let result = leapfrog_union(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn leapfrog_union_with_empty_input() {
        let a = Index::from_i64(vec![1, 2]);
        let b = Index::new(Vec::new());
        let result = leapfrog_union(&[&a, &b]);
        assert_eq!(
            result.labels(),
            &[IndexLabel::Int64(1), IndexLabel::Int64(2)]
        );
    }

    #[test]
    fn leapfrog_intersection_three_indexes() {
        let a = Index::from_i64(vec![1, 2, 3, 4, 5]);
        let b = Index::from_i64(vec![2, 3, 5, 7]);
        let c = Index::from_i64(vec![3, 5, 8]);
        let result = leapfrog_intersection(&[&a, &b, &c]);
        assert_eq!(
            result.labels(),
            &[IndexLabel::Int64(3), IndexLabel::Int64(5)]
        );
    }

    #[test]
    fn leapfrog_intersection_disjoint() {
        let a = Index::from_i64(vec![1, 2]);
        let b = Index::from_i64(vec![3, 4]);
        let result = leapfrog_intersection(&[&a, &b]);
        assert!(result.is_empty());
    }

    #[test]
    fn leapfrog_intersection_identical() {
        let a = Index::from_i64(vec![1, 2, 3]);
        let b = Index::from_i64(vec![1, 2, 3]);
        let result = leapfrog_intersection(&[&a, &b]);
        assert_eq!(
            result.labels(),
            &[
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
                IndexLabel::Int64(3),
            ]
        );
    }

    #[test]
    fn leapfrog_intersection_with_unsorted_input() {
        let a = Index::from_i64(vec![5, 3, 1, 4, 2]);
        let b = Index::from_i64(vec![4, 2, 6, 1]);
        let result = leapfrog_intersection(&[&a, &b]);
        assert_eq!(
            result.labels(),
            &[
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
                IndexLabel::Int64(4),
            ]
        );
    }

    #[test]
    fn leapfrog_intersection_empty_input() {
        let a = Index::from_i64(vec![1, 2, 3]);
        let b = Index::new(Vec::new());
        let result = leapfrog_intersection(&[&a, &b]);
        assert!(result.is_empty());
    }

    #[test]
    fn multi_way_align_three_indexes() {
        let a = Index::from_i64(vec![1, 3]);
        let b = Index::from_i64(vec![2, 3]);
        let c = Index::from_i64(vec![1, 2]);
        let plan = multi_way_align(&[&a, &b, &c]);
        assert_eq!(
            plan.union_index.labels(),
            &[
                IndexLabel::Int64(1),
                IndexLabel::Int64(3),
                IndexLabel::Int64(2),
            ]
        );
        assert_eq!(plan.positions.len(), 3);
        // a has 1 at pos 0, 3 at pos 1, no 2
        assert_eq!(plan.positions[0], vec![Some(0), Some(1), None]);
        // b has no 1, 3 at pos 1, 2 at pos 0
        assert_eq!(plan.positions[1], vec![None, Some(1), Some(0)]);
        // c has 1 at pos 0, no 3, 2 at pos 1
        assert_eq!(plan.positions[2], vec![Some(0), None, Some(1)]);
    }

    #[test]
    fn multi_way_align_empty() {
        let plan = multi_way_align(&[]);
        assert!(plan.union_index.is_empty());
        assert!(plan.positions.is_empty());
    }

    #[test]
    fn multi_way_align_isomorphic_with_pairwise() {
        // AG-11 contract: multi-way union produces same label set as
        // iterative pairwise union (associativity + commutativity).
        let a = Index::from_i64(vec![1, 4, 7]);
        let b = Index::from_i64(vec![2, 4, 8]);
        let c = Index::from_i64(vec![3, 7, 8]);

        let multi = leapfrog_union(&[&a, &b, &c]);

        // Iterative pairwise
        let ab = a.union_with(&b);
        let abc = ab.union_with(&c);
        let pairwise = abc.sort_values();

        assert_eq!(multi.labels(), pairwise.labels());
    }

    #[test]
    fn leapfrog_union_utf8_labels() {
        let a = Index::new(vec!["c".into(), "a".into()]);
        let b = Index::new(vec!["b".into(), "d".into()]);
        let result = leapfrog_union(&[&a, &b]);
        assert_eq!(
            result.labels(),
            &["a".into(), "b".into(), "c".into(), "d".into()]
        );
    }

    #[test]
    fn leapfrog_large_multi_way() {
        // 5 indexes, each 1000 labels, overlapping ranges
        let indexes: Vec<Index> = (0..5)
            .map(|i| {
                let start = i * 200;
                let end = start + 1000;
                Index::from_i64((start..end).collect())
            })
            .collect();
        let refs: Vec<&Index> = indexes.iter().collect();

        let union = leapfrog_union(&refs);
        // Range is 0..1800 (0-999, 200-1199, 400-1399, 600-1599, 800-1799)
        assert_eq!(union.len(), 1800);

        let intersection = leapfrog_intersection(&refs);
        // Intersection is 800..999 (all 5 overlap)
        assert_eq!(intersection.len(), 200);
    }

    // === AG-11-T: Full test plan (bd-2t5e.17) ===

    #[test]
    fn ag11t_two_sorted_identical() {
        let a = Index::from_i64(vec![1, 2, 3]);
        let b = Index::from_i64(vec![1, 2, 3]);
        let result = leapfrog_union(&[&a, &b]);
        assert_eq!(
            result.labels(),
            &[
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
                IndexLabel::Int64(3)
            ]
        );
        let plan = multi_way_align(&[&a, &b]);
        // Both map to identity positions
        assert_eq!(plan.positions[0], vec![Some(0), Some(1), Some(2)]);
        assert_eq!(plan.positions[1], vec![Some(0), Some(1), Some(2)]);
        eprintln!("[AG-11-T] two_sorted_identical | in=[3,3] out=3 | PASS");
    }

    #[test]
    fn ag11t_two_sorted_disjoint() {
        let a = Index::from_i64(vec![1, 2, 3]);
        let b = Index::from_i64(vec![4, 5, 6]);
        let result = leapfrog_union(&[&a, &b]);
        assert_eq!(result.len(), 6);
        assert_eq!(result.labels()[0], IndexLabel::Int64(1));
        assert_eq!(result.labels()[5], IndexLabel::Int64(6));
        eprintln!("[AG-11-T] two_sorted_disjoint | in=[3,3] out=6 | PASS");
    }

    #[test]
    fn ag11t_two_sorted_overlapping_with_positions() {
        let a = Index::from_i64(vec![1, 3, 5]);
        let b = Index::from_i64(vec![2, 3, 4]);
        let plan = multi_way_align(&[&a, &b]);
        assert_eq!(
            plan.union_index.labels(),
            &[
                IndexLabel::Int64(1),
                IndexLabel::Int64(3),
                IndexLabel::Int64(5),
                IndexLabel::Int64(2),
                IndexLabel::Int64(4),
            ]
        );
        assert_eq!(
            plan.positions[0],
            vec![Some(0), Some(1), Some(2), None, None]
        );
        assert_eq!(
            plan.positions[1],
            vec![None, Some(1), None, Some(0), Some(2)]
        );
        eprintln!("[AG-11-T] two_sorted_overlapping | in=[3,3] out=5 | PASS");
    }

    #[test]
    fn ag11t_five_way_union_vs_pairwise() {
        let indexes: Vec<Index> = (0..5)
            .map(|i| Index::from_i64(vec![i * 10, i * 10 + 1, i * 10 + 2]))
            .collect();
        let refs: Vec<&Index> = indexes.iter().collect();

        let leapfrog = leapfrog_union(&refs);

        // Iterative pairwise
        let mut pairwise = indexes[0].clone();
        for idx in &indexes[1..] {
            pairwise = pairwise.union_with(idx);
        }
        let pairwise = pairwise.sort_values();

        assert_eq!(leapfrog.labels(), pairwise.labels());
        eprintln!(
            "[AG-11-T] five_way_union_vs_pairwise | in=[3x5] out={} | PASS",
            leapfrog.len()
        );
    }

    #[test]
    fn ag11t_single_element_indexes() {
        let indexes: Vec<Index> = (0..10).map(|i| Index::from_i64(vec![i])).collect();
        let refs: Vec<&Index> = indexes.iter().collect();
        let result = leapfrog_union(&refs);
        assert_eq!(result.len(), 10);
        for (i, label) in result.labels().iter().enumerate() {
            assert_eq!(*label, IndexLabel::Int64(i as i64));
        }
        eprintln!("[AG-11-T] single_element_indexes | in=[1x10] out=10 | PASS");
    }

    #[test]
    fn ag11t_all_same_labels() {
        let base = Index::from_i64(vec![1, 2, 3]);
        let refs: Vec<&Index> = (0..5).map(|_| &base).collect();
        let plan = multi_way_align(&refs);
        assert_eq!(
            plan.union_index.labels(),
            &[
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
                IndexLabel::Int64(3)
            ]
        );
        // All 5 inputs should have identity positions
        for pos_vec in &plan.positions {
            assert_eq!(*pos_vec, vec![Some(0), Some(1), Some(2)]);
        }
        eprintln!("[AG-11-T] all_same_labels | in=[3x5] out=3 | PASS");
    }

    #[test]
    fn ag11t_iso_associativity() {
        let a = Index::from_i64(vec![1, 4, 7, 10]);
        let b = Index::from_i64(vec![2, 4, 8, 10]);
        let c = Index::from_i64(vec![3, 7, 8, 10]);

        let leapfrog_result = leapfrog_union(&[&a, &b, &c]);

        // union(A, union(B, C))
        let bc = b.union_with(&c).sort_values();
        let a_bc = a.union_with(&bc).sort_values();

        // union(union(A, B), C)
        let ab = a.union_with(&b).sort_values();
        let ab_c = ab.union_with(&c).sort_values();

        assert_eq!(leapfrog_result.labels(), a_bc.labels());
        assert_eq!(leapfrog_result.labels(), ab_c.labels());
        eprintln!("[AG-11-T] iso_associativity | verified | PASS");
    }

    #[test]
    fn ag11t_iso_commutativity() {
        let a = Index::from_i64(vec![1, 5, 9]);
        let b = Index::from_i64(vec![2, 5, 8]);
        let c = Index::from_i64(vec![3, 5, 7]);

        let abc = leapfrog_union(&[&a, &b, &c]);
        let cab = leapfrog_union(&[&c, &a, &b]);
        let bca = leapfrog_union(&[&b, &c, &a]);

        // All orderings produce same sorted output
        assert_eq!(abc.labels(), cab.labels());
        assert_eq!(abc.labels(), bca.labels());
        eprintln!("[AG-11-T] iso_commutativity | verified | PASS");
    }

    // ── Index: min/max/argmin/argmax ──

    #[test]
    fn index_min_max_int() {
        let idx = Index::new(vec![3_i64.into(), 1_i64.into(), 2_i64.into()]);
        assert_eq!(idx.min(), Some(&IndexLabel::Int64(1)));
        assert_eq!(idx.max(), Some(&IndexLabel::Int64(3)));
        assert_eq!(idx.argmin(), Some(1));
        assert_eq!(idx.argmax(), Some(0));
    }

    #[test]
    fn index_min_max_utf8() {
        let idx = Index::new(vec!["c".into(), "a".into(), "b".into()]);
        assert_eq!(idx.min(), Some(&IndexLabel::Utf8("a".into())));
        assert_eq!(idx.max(), Some(&IndexLabel::Utf8("c".into())));
        assert_eq!(idx.argmin(), Some(1));
        assert_eq!(idx.argmax(), Some(0));
    }

    #[test]
    fn index_min_max_empty() {
        let idx = Index::new(vec![]);
        assert_eq!(idx.min(), None);
        assert_eq!(idx.max(), None);
        assert_eq!(idx.argmin(), None);
        assert_eq!(idx.argmax(), None);
    }

    #[test]
    fn index_nunique() {
        let idx = Index::new(vec![1_i64.into(), 2_i64.into(), 1_i64.into()]);
        assert_eq!(idx.nunique(), 2);
    }

    #[test]
    fn index_nunique_dropna_false_counts_timedelta_nat_once() {
        let idx = Index::from_timedelta64(vec![Timedelta::NAT, Timedelta::NAT, 5]);
        assert_eq!(idx.nunique(), 1);
        assert_eq!(idx.nunique_with_dropna(false), 2);
    }

    #[test]
    fn index_nunique_dropna_false_counts_datetime_nat_once() {
        let idx = Index::new(vec![
            IndexLabel::Datetime64(i64::MIN),
            IndexLabel::Datetime64(i64::MIN),
            IndexLabel::Datetime64(1_700_000_000_000_000_000),
        ]);
        assert_eq!(idx.nunique(), 1);
        assert_eq!(idx.nunique_with_dropna(false), 2);
    }

    // ── Index: map/rename/drop/astype ──

    #[test]
    fn index_map() {
        let idx = Index::new(vec![1_i64.into(), 2_i64.into(), 3_i64.into()]);
        let mapped = idx.map(|l| match l {
            IndexLabel::Int64(v) => IndexLabel::Int64(v * 10),
            other => other.clone(),
        });
        assert_eq!(mapped.labels()[0], IndexLabel::Int64(10));
        assert_eq!(mapped.labels()[2], IndexLabel::Int64(30));
    }

    #[test]
    fn index_drop_labels() {
        let idx = Index::new(vec!["a".into(), "b".into(), "c".into()]);
        let dropped = idx.drop_labels(&["b".into()]);
        assert_eq!(dropped.len(), 2);
        assert_eq!(dropped.labels()[0], IndexLabel::Utf8("a".into()));
        assert_eq!(dropped.labels()[1], IndexLabel::Utf8("c".into()));
    }

    #[test]
    fn index_astype_str() {
        let idx = Index::new(vec![1_i64.into(), 2_i64.into()]);
        let str_idx = idx.astype_str();
        assert_eq!(str_idx.labels()[0], IndexLabel::Utf8("1".into()));
        assert_eq!(str_idx.labels()[1], IndexLabel::Utf8("2".into()));
    }

    #[test]
    fn index_astype_int() {
        let idx = Index::new(vec![
            IndexLabel::Utf8("10".into()),
            IndexLabel::Utf8("20".into()),
        ]);
        let int_idx = idx.astype_int();
        assert_eq!(int_idx.labels()[0], IndexLabel::Int64(10));
        assert_eq!(int_idx.labels()[1], IndexLabel::Int64(20));
    }

    #[test]
    fn index_isna_notna() {
        let idx = Index::new(vec![1_i64.into(), 2_i64.into()]);
        assert_eq!(idx.isna(), vec![false, false]);
        assert_eq!(idx.notna(), vec![true, true]);
    }

    #[test]
    fn index_isna_notna_detects_datetimelike_nat() {
        let datetime_idx = Index::new(vec![
            IndexLabel::Datetime64(i64::MIN),
            IndexLabel::Datetime64(1_700_000_000_000_000_000),
        ]);
        assert_eq!(datetime_idx.isna(), vec![true, false]);
        assert_eq!(datetime_idx.notna(), vec![false, true]);

        let timedelta_idx = Index::from_timedelta64(vec![Timedelta::NAT, 5]);
        assert_eq!(timedelta_idx.isna(), vec![true, false]);
        assert_eq!(timedelta_idx.notna(), vec![false, true]);
    }

    #[test]
    fn index_fillna_replaces_datetime_nat_and_preserves_name() {
        let idx = Index::new(vec![
            IndexLabel::Datetime64(i64::MIN),
            IndexLabel::Datetime64(1_700_000_000_000_000_000),
            IndexLabel::Datetime64(i64::MIN),
        ])
        .set_name("when");

        let filled = idx.fillna(&IndexLabel::Datetime64(1_800_000_000_000_000_000));

        assert_eq!(
            filled.labels(),
            &[
                IndexLabel::Datetime64(1_800_000_000_000_000_000),
                IndexLabel::Datetime64(1_700_000_000_000_000_000),
                IndexLabel::Datetime64(1_800_000_000_000_000_000),
            ]
        );
        assert_eq!(filled.name(), Some("when"));
    }

    #[test]
    fn index_fillna_replaces_timedelta_nat() {
        let idx = Index::from_timedelta64(vec![Timedelta::NAT, 5, Timedelta::NAT]);

        let filled = idx.fillna(&IndexLabel::Timedelta64(42));

        assert_eq!(
            filled.labels(),
            &[
                IndexLabel::Timedelta64(42),
                IndexLabel::Timedelta64(5),
                IndexLabel::Timedelta64(42),
            ]
        );
    }

    #[test]
    fn index_dropna_removes_missing_and_preserves_name() {
        let idx =
            Index::from_timedelta64(vec![1, Timedelta::NAT, 3, Timedelta::NAT, 5]).set_name("t");
        let dropped = idx.dropna();
        assert_eq!(
            dropped.labels(),
            &[
                IndexLabel::Timedelta64(1),
                IndexLabel::Timedelta64(3),
                IndexLabel::Timedelta64(5),
            ]
        );
        assert_eq!(dropped.name(), Some("t"));
    }

    #[test]
    fn index_dropna_all_present_is_noop() {
        let idx = Index::from_i64(vec![1, 2, 3]);
        let dropped = idx.dropna();
        assert_eq!(
            dropped.labels(),
            &[
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
                IndexLabel::Int64(3),
            ]
        );
    }

    #[test]
    fn index_insert_at_middle_position() {
        let idx = Index::from_i64(vec![1, 3, 4]);
        let result = idx.insert(1, IndexLabel::Int64(2)).unwrap();
        assert_eq!(
            result.labels(),
            &[
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
                IndexLabel::Int64(3),
                IndexLabel::Int64(4),
            ]
        );
    }

    #[test]
    fn index_insert_at_end_appends() {
        let idx = Index::from_i64(vec![1, 2]);
        let result = idx.insert(2, IndexLabel::Int64(3)).unwrap();
        assert_eq!(
            result.labels(),
            &[
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
                IndexLabel::Int64(3),
            ]
        );
    }

    #[test]
    fn index_insert_past_end_errors() {
        let idx = Index::from_i64(vec![1, 2]);
        let err = idx.insert(5, IndexLabel::Int64(9)).unwrap_err();
        assert!(matches!(err, crate::IndexError::OutOfBounds { .. }));
    }

    #[test]
    fn index_delete_removes_position() {
        let idx = Index::from_i64(vec![10, 20, 30]).set_name("k");
        let result = idx.delete(1).unwrap();
        assert_eq!(
            result.labels(),
            &[IndexLabel::Int64(10), IndexLabel::Int64(30)]
        );
        assert_eq!(result.name(), Some("k"));
    }

    #[test]
    fn index_delete_out_of_bounds_errors() {
        let idx = Index::from_i64(vec![1]);
        let err = idx.delete(1).unwrap_err();
        assert!(matches!(err, crate::IndexError::OutOfBounds { .. }));
    }

    #[test]
    fn index_append_concatenates() {
        let a = Index::from_i64(vec![1, 2]).set_name("left");
        let b = Index::from_i64(vec![3, 4]);
        let result = a.append(&b);
        assert_eq!(
            result.labels(),
            &[
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
                IndexLabel::Int64(3),
                IndexLabel::Int64(4),
            ]
        );
        assert_eq!(result.name(), Some("left"));
    }

    #[test]
    fn index_append_empty_is_noop() {
        let a = Index::from_i64(vec![1, 2]);
        let empty = Index::new(Vec::new());
        let result = a.append(&empty);
        assert_eq!(result.labels(), a.labels());
    }

    #[test]
    fn index_repeat_duplicates_each_label() {
        let idx = Index::from_i64(vec![1, 2, 3]).set_name("k");
        let result = idx.repeat(2);
        assert_eq!(
            result.labels(),
            &[
                IndexLabel::Int64(1),
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
                IndexLabel::Int64(2),
                IndexLabel::Int64(3),
                IndexLabel::Int64(3),
            ]
        );
        assert_eq!(result.name(), Some("k"));
    }

    #[test]
    fn index_repeat_zero_yields_empty() {
        let idx = Index::from_i64(vec![1, 2, 3]);
        let result = idx.repeat(0);
        assert!(result.labels().is_empty());
    }

    #[test]
    fn index_repeat_one_is_clone() {
        let idx = Index::from_i64(vec![1, 2]);
        let result = idx.repeat(1);
        assert_eq!(result.labels(), idx.labels());
    }

    #[test]
    fn index_equals_same_labels_ignores_name() {
        let a = Index::from_i64(vec![1, 2, 3]).set_name("x");
        let b = Index::from_i64(vec![1, 2, 3]).set_name("y");
        assert!(a.equals(&b));
    }

    #[test]
    fn index_equals_differing_labels_false() {
        let a = Index::from_i64(vec![1, 2, 3]);
        let b = Index::from_i64(vec![1, 2]);
        assert!(!a.equals(&b));
    }

    #[test]
    fn index_identical_requires_matching_name() {
        let a = Index::from_i64(vec![1, 2]).set_name("x");
        let b = Index::from_i64(vec![1, 2]).set_name("y");
        assert!(a.equals(&b));
        assert!(!a.identical(&b));
        let c = Index::from_i64(vec![1, 2]).set_name("x");
        assert!(a.identical(&c));
    }

    #[test]
    fn index_value_counts_sorts_by_descending_count() {
        let idx = Index::new(vec![
            "a".into(),
            "b".into(),
            "a".into(),
            "c".into(),
            "a".into(),
            "b".into(),
        ]);
        let counts = idx.value_counts();
        assert_eq!(counts[0].0, IndexLabel::Utf8("a".into()));
        assert_eq!(counts[0].1, 3);
        assert_eq!(counts[1].0, IndexLabel::Utf8("b".into()));
        assert_eq!(counts[1].1, 2);
        assert_eq!(counts[2].0, IndexLabel::Utf8("c".into()));
        assert_eq!(counts[2].1, 1);
    }

    #[test]
    fn index_value_counts_empty() {
        let idx = Index::new(Vec::<IndexLabel>::new());
        assert!(idx.value_counts().is_empty());
    }

    #[test]
    fn index_value_counts_drops_missing_by_default() {
        let idx = Index::new(vec![
            IndexLabel::Datetime64(i64::MIN),
            IndexLabel::Utf8("a".into()),
            IndexLabel::Utf8("a".into()),
            IndexLabel::Datetime64(i64::MIN),
        ]);

        let counts = idx.value_counts();
        assert_eq!(counts, vec![(IndexLabel::Utf8("a".into()), 2)]);
    }

    #[test]
    fn index_value_counts_with_options_preserves_first_seen_order_when_unsorted() {
        let idx = Index::new(vec![
            IndexLabel::Datetime64(i64::MIN),
            IndexLabel::Utf8("b".into()),
            IndexLabel::Utf8("a".into()),
            IndexLabel::Utf8("b".into()),
        ]);

        let counts = idx.value_counts_with_options(false, false, false, false);
        assert_eq!(
            counts,
            vec![
                (IndexLabel::Datetime64(i64::MIN), Scalar::Int64(1)),
                (IndexLabel::Utf8("b".into()), Scalar::Int64(2)),
                (IndexLabel::Utf8("a".into()), Scalar::Int64(1)),
            ]
        );
    }

    #[test]
    fn index_value_counts_with_options_normalize_excludes_missing_from_denominator() {
        let idx = Index::new(vec![
            IndexLabel::Int64(1),
            IndexLabel::Int64(1),
            IndexLabel::Int64(2),
            IndexLabel::Datetime64(i64::MIN),
        ]);

        let counts = idx.value_counts_with_options(true, true, false, true);
        assert!(matches!(
            counts.as_slice(),
            [
                (IndexLabel::Int64(1), Scalar::Float64(_)),
                (IndexLabel::Int64(2), Scalar::Float64(_))
            ]
        ));
        let [
            (IndexLabel::Int64(1), Scalar::Float64(first)),
            (IndexLabel::Int64(2), Scalar::Float64(second)),
        ] = counts.as_slice()
        else {
            return;
        };
        assert!((first - (2.0 / 3.0)).abs() < 1e-12);
        assert!((second - (1.0 / 3.0)).abs() < 1e-12);
    }

    #[test]
    fn index_shift_positive_pads_left() {
        let idx = Index::from_i64(vec![1, 2, 3, 4]).set_name("k");
        let shifted = idx.shift(2, IndexLabel::Int64(-1));
        assert_eq!(
            shifted.labels(),
            &[
                IndexLabel::Int64(-1),
                IndexLabel::Int64(-1),
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
            ]
        );
        assert_eq!(shifted.name(), Some("k"));
    }

    #[test]
    fn index_shift_negative_pads_right() {
        let idx = Index::from_i64(vec![1, 2, 3, 4]);
        let shifted = idx.shift(-1, IndexLabel::Int64(0));
        assert_eq!(
            shifted.labels(),
            &[
                IndexLabel::Int64(2),
                IndexLabel::Int64(3),
                IndexLabel::Int64(4),
                IndexLabel::Int64(0),
            ]
        );
    }

    #[test]
    fn index_shift_zero_is_clone() {
        let idx = Index::from_i64(vec![1, 2, 3]);
        let shifted = idx.shift(0, IndexLabel::Int64(-1));
        assert_eq!(shifted.labels(), idx.labels());
    }

    #[test]
    fn index_shift_larger_than_len_fills_all() {
        let idx = Index::from_i64(vec![1, 2, 3]);
        let shifted = idx.shift(10, IndexLabel::Int64(-1));
        assert_eq!(
            shifted.labels(),
            &[
                IndexLabel::Int64(-1),
                IndexLabel::Int64(-1),
                IndexLabel::Int64(-1),
            ]
        );
    }

    #[test]
    fn index_any_all_basic() {
        let idx = Index::from_i64(vec![0, 0, 1]);
        assert!(idx.any());
        assert!(!idx.all());

        let all_nonzero = Index::from_i64(vec![1, 2, 3]);
        assert!(all_nonzero.all());
        assert!(all_nonzero.any());

        let all_zero = Index::from_i64(vec![0, 0]);
        assert!(!all_zero.any());
        assert!(!all_zero.all());
    }

    #[test]
    fn index_all_empty_is_true() {
        let idx = Index::new(Vec::<IndexLabel>::new());
        assert!(idx.all());
        assert!(!idx.any());
    }

    #[test]
    fn index_any_string_nonempty_truthy() {
        let idx = Index::new(vec!["".into(), "".into(), "x".into()]);
        assert!(idx.any());
        assert!(!idx.all());
    }

    #[test]
    fn index_to_list_returns_owned_labels() {
        let idx = Index::from_i64(vec![1, 2, 3]);
        assert_eq!(
            idx.to_list(),
            vec![
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
                IndexLabel::Int64(3),
            ]
        );
    }

    #[test]
    fn index_format_stringifies_labels() {
        let idx = Index::new(vec![
            IndexLabel::Int64(10),
            IndexLabel::Utf8("abc".into()),
            IndexLabel::Int64(-5),
        ]);
        assert_eq!(idx.format(), vec!["10", "abc", "-5"]);
    }

    #[test]
    fn index_putmask_replaces_true_positions() {
        let idx = Index::from_i64(vec![1, 2, 3, 4]).set_name("k");
        let cond = vec![false, true, false, true];
        let replaced = idx.putmask(&cond, &IndexLabel::Int64(0));
        assert_eq!(
            replaced.labels(),
            &[
                IndexLabel::Int64(1),
                IndexLabel::Int64(0),
                IndexLabel::Int64(3),
                IndexLabel::Int64(0),
            ]
        );
        assert_eq!(replaced.name(), Some("k"));
    }

    #[test]
    fn index_putmask_short_cond_leaves_tail_unchanged() {
        let idx = Index::from_i64(vec![1, 2, 3, 4]);
        // cond shorter than index — trailing positions keep original
        // labels (matches pandas lenient alignment).
        let cond = vec![true];
        let replaced = idx.putmask(&cond, &IndexLabel::Int64(-1));
        assert_eq!(
            replaced.labels(),
            &[
                IndexLabel::Int64(-1),
                IndexLabel::Int64(2),
                IndexLabel::Int64(3),
                IndexLabel::Int64(4),
            ]
        );
    }

    #[test]
    fn index_putmask_empty_cond_is_noop() {
        let idx = Index::from_i64(vec![1, 2]);
        let replaced = idx.putmask(&[], &IndexLabel::Int64(0));
        assert_eq!(replaced.labels(), idx.labels());
    }

    #[test]
    fn index_asof_finds_largest_not_exceeding() {
        let idx = Index::from_i64(vec![1, 3, 5, 7]);
        assert_eq!(idx.asof(&IndexLabel::Int64(4)), Some(IndexLabel::Int64(3)));
        assert_eq!(idx.asof(&IndexLabel::Int64(5)), Some(IndexLabel::Int64(5)));
        assert_eq!(idx.asof(&IndexLabel::Int64(7)), Some(IndexLabel::Int64(7)));
        assert_eq!(
            idx.asof(&IndexLabel::Int64(100)),
            Some(IndexLabel::Int64(7))
        );
    }

    #[test]
    fn index_asof_before_first_returns_none() {
        let idx = Index::from_i64(vec![5, 10]);
        assert_eq!(idx.asof(&IndexLabel::Int64(0)), None);
    }

    #[test]
    fn index_searchsorted_left_right() {
        let idx = Index::from_i64(vec![1, 2, 2, 5]);
        assert_eq!(idx.searchsorted(&IndexLabel::Int64(2), "left").unwrap(), 1);
        assert_eq!(idx.searchsorted(&IndexLabel::Int64(2), "right").unwrap(), 3);
        assert_eq!(idx.searchsorted(&IndexLabel::Int64(0), "left").unwrap(), 0);
        assert_eq!(idx.searchsorted(&IndexLabel::Int64(6), "left").unwrap(), 4);
    }

    #[test]
    fn index_searchsorted_rejects_invalid_side() {
        let idx = Index::from_i64(vec![1]);
        assert!(idx.searchsorted(&IndexLabel::Int64(0), "middle").is_err());
    }

    #[test]
    fn index_memory_usage_counts_fixed_width() {
        let idx = Index::from_i64(vec![1, 2, 3]);
        let shallow = idx.memory_usage(false);
        assert_eq!(shallow, 24); // 3 * 8
        // deep is identical for fixed-width types.
        assert_eq!(idx.memory_usage(true), 24);
    }

    #[test]
    fn index_memory_usage_deep_counts_utf8_bytes() {
        let idx = Index::new(vec![
            IndexLabel::Utf8("hi".into()),
            IndexLabel::Utf8("world".into()),
        ]);
        let shallow = idx.memory_usage(false);
        let deep = idx.memory_usage(true);
        // deep - shallow == sum of string byte lengths
        assert_eq!(deep - shallow, 7);
    }

    #[test]
    fn index_nlevels_flat_index_is_one() {
        let idx = Index::from_i64(vec![1, 2]);
        assert_eq!(idx.nlevels(), 1);
    }

    #[test]
    fn index_where_cond() {
        let idx = Index::new(vec!["a".into(), "b".into(), "c".into()]);
        let cond = vec![true, false, true];
        let result = idx.where_cond(&cond, &"X".into());
        assert_eq!(result.labels()[0], IndexLabel::Utf8("a".into()));
        assert_eq!(result.labels()[1], IndexLabel::Utf8("X".into()));
        assert_eq!(result.labels()[2], IndexLabel::Utf8("c".into()));
    }

    #[test]
    fn index_a31qh_conversion_aliases_materialize_labels() {
        let idx = Index::new(vec!["a".into(), "b".into()]).set_name("key");
        let labels = vec![IndexLabel::from("a"), IndexLabel::from("b")];

        assert_eq!(idx.tolist(), labels);
        assert_eq!(idx.to_numpy(), labels);
        assert_eq!(idx.array(), labels);
        assert_eq!(idx.values(), labels);
        assert_eq!(idx.ravel(), labels);
        assert_eq!(idx.view(), idx);
        assert_eq!(idx.transpose(), idx);
        assert_eq!(idx.T(), idx);
        assert_eq!(
            idx.to_frame(),
            vec![vec![IndexLabel::from("a")], vec![IndexLabel::from("b")]]
        );
        assert_eq!(
            idx.to_series(),
            vec![
                (IndexLabel::from("a"), IndexLabel::from("a")),
                (IndexLabel::from("b"), IndexLabel::from("b")),
            ]
        );
    }

    #[test]
    fn index_a31qh_dtype_metadata_and_type_checks() {
        let ints = Index::from_i64(vec![1, 2, 3]);
        assert_eq!(ints.dtype(), "int64");
        assert_eq!(ints.dtypes(), vec!["int64"]);
        assert_eq!(ints.inferred_type(), "integer");
        assert!(ints.holds_integer());
        assert!(ints.is_integer());
        assert!(ints.is_numeric());
        assert!(!ints.is_object());
        assert_eq!(ints.ndim(), 1);
        assert_eq!(ints.shape(), (3,));
        assert_eq!(ints.size(), 3);
        assert_eq!(ints.nbytes(), ints.memory_usage(false));
        assert!(!ints.empty());
        assert_eq!(
            Index::from_i64(vec![42]).item().unwrap(),
            IndexLabel::Int64(42)
        );
        assert!(ints.item().is_err());

        let mixed = Index::new(vec![
            IndexLabel::Int64(1),
            IndexLabel::Utf8("x".into()),
            IndexLabel::Datetime64(i64::MIN),
        ]);
        assert_eq!(mixed.dtype(), "object");
        assert_eq!(mixed.inferred_type(), "mixed");
        assert!(mixed.is_object());
        assert!(mixed.hasnans());
        assert_eq!(mixed.isnull(), mixed.isna());
        assert_eq!(mixed.notnull(), mixed.notna());
        assert!(!mixed.is_boolean());
        assert!(!mixed.is_categorical());
        assert!(!mixed.is_floating());
        assert!(!mixed.is_interval());
        assert_eq!(mixed.infer_objects(), mixed);
        assert!(ints.is_(&ints));
        assert!(!ints.is_(&Index::from_i64(vec![1, 2, 3])));
    }

    #[test]
    fn index_a31qh_factorize_reindex_and_non_unique_indexer() {
        let idx = Index::new(vec![
            IndexLabel::Utf8("a".into()),
            IndexLabel::Utf8("b".into()),
            IndexLabel::Utf8("a".into()),
            IndexLabel::Datetime64(i64::MIN),
        ])
        .set_name("letters");

        let (codes, uniques) = idx.factorize();
        assert_eq!(codes, vec![0, 1, 0, -1]);
        assert_eq!(
            uniques.labels(),
            &[IndexLabel::from("a"), IndexLabel::from("b")]
        );
        assert_eq!(uniques.name(), Some("letters"));

        let target = Index::new(vec![
            IndexLabel::Utf8("a".into()),
            IndexLabel::Utf8("z".into()),
            IndexLabel::Utf8("b".into()),
        ]);
        assert_eq!(idx.get_indexer_for(&target), vec![Some(0), None, Some(1)]);
        assert_eq!(
            idx.get_indexer_non_unique(&target),
            (vec![0, 2, -1, 1], vec![1])
        );

        let (reindexed, positions) = idx.reindex(&target);
        assert_eq!(reindexed, target);
        assert_eq!(positions, vec![Some(0), None, Some(1)]);
    }

    #[test]
    fn index_a31qh_set_sort_slice_and_level_aliases() {
        let idx = Index::from_i64(vec![3, 1, 2]).set_name("n");
        let sorted = idx.sort();
        assert_eq!(
            sorted.labels(),
            &[
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
                IndexLabel::Int64(3),
            ]
        );
        let (sortlevel, order) = idx.sortlevel();
        assert_eq!(sortlevel, sorted);
        assert_eq!(order, vec![1, 2, 0]);

        let other = Index::from_i64(vec![2, 4]);
        assert_eq!(idx.union(&other), idx.union_with(&other));
        assert_eq!(
            idx.drop(&[IndexLabel::Int64(1)]),
            idx.drop_labels(&[IndexLabel::Int64(1)])
        );
        assert_eq!(idx.copy(), idx);
        assert_eq!(
            idx.where_(&[true, false, true], &IndexLabel::Int64(0))
                .labels()[1],
            IndexLabel::Int64(0)
        );
        assert_eq!(idx.get_level_values(0).unwrap(), idx);
        assert!(idx.get_level_values(1).is_err());
        assert!(idx.droplevel(0).is_err());

        let sorted_lookup = Index::from_i64(vec![1, 2, 2, 4]);
        assert_eq!(
            sorted_lookup
                .get_slice_bound(&IndexLabel::Int64(2), "left")
                .unwrap(),
            1
        );
        assert_eq!(
            sorted_lookup
                .slice_locs(Some(&IndexLabel::Int64(2)), Some(&IndexLabel::Int64(4)))
                .unwrap(),
            (1, 4)
        );
        assert_eq!(
            sorted_lookup
                .slice_indexer(Some(&IndexLabel::Int64(2)), Some(&IndexLabel::Int64(2)))
                .unwrap(),
            (1, 3)
        );
    }

    #[test]
    fn index_a31qh_astype_str_groupby_join_asof_and_diff() {
        let idx = Index::new(vec![
            IndexLabel::Utf8("Alpha".into()),
            IndexLabel::Utf8("beta".into()),
            IndexLabel::Int64(7),
        ]);
        assert_eq!(
            idx.r#str().lower(),
            vec![Some("alpha".to_owned()), Some("beta".to_owned()), None]
        );
        assert_eq!(
            idx.r#str().upper(),
            vec![Some("ALPHA".to_owned()), Some("BETA".to_owned()), None]
        );
        assert_eq!(
            idx.r#str().contains("ta"),
            vec![Some(false), Some(true), None]
        );
        assert_eq!(idx.r#str().len(), vec![Some(5), Some(4), None]);
        assert_eq!(idx.r#str().is_empty(), vec![Some(false), Some(false), None]);
        assert!(idx.astype("object").is_ok());
        assert!(idx.astype("float64").is_err());

        let grouped = Index::new(vec!["a".into(), "b".into(), "a".into()]).groupby();
        assert_eq!(grouped[&IndexLabel::from("a")], vec![0, 2]);
        assert_eq!(grouped[&IndexLabel::from("b")], vec![1]);

        let left = Index::from_i64(vec![1, 2, 3]);
        let right = Index::from_i64(vec![2, 4]);
        assert_eq!(
            left.join(&right, "inner").unwrap(),
            left.intersection(&right)
        );
        assert_eq!(left.join(&right, "outer").unwrap(), left.union_with(&right));
        assert_eq!(left.join(&right, "left").unwrap(), left);
        assert_eq!(left.join(&right, "right").unwrap(), right);
        assert!(left.join(&right, "sideways").is_err());

        let sorted = Index::from_i64(vec![1, 3, 5, 7]);
        let probes = Index::from_i64(vec![0, 3, 4, 8]);
        assert_eq!(
            sorted.asof_locs(&probes, None),
            vec![None, Some(1), Some(1), Some(3)]
        );
        assert_eq!(
            sorted.asof_locs(&probes, Some(&[true, false, true, true])),
            vec![None, Some(0), Some(0), Some(3)]
        );

        assert_eq!(
            sorted.diff(1),
            vec![
                None,
                Some(IndexLabel::Int64(2)),
                Some(IndexLabel::Int64(2)),
                Some(IndexLabel::Int64(2)),
            ]
        );
        let datetimes = Index::from_datetime64(vec![10, 25]);
        assert_eq!(
            datetimes.diff(1),
            vec![None, Some(IndexLabel::Timedelta64(15))]
        );
    }

    // ── Index name tests ────────────────────────────────────────────

    #[test]
    fn index_name_default_none() {
        let idx = Index::new(vec![1_i64.into(), 2_i64.into()]);
        assert_eq!(idx.name(), None);
    }

    #[test]
    fn index_set_name() {
        let idx = Index::new(vec![1_i64.into(), 2_i64.into()]);
        let named = idx.set_name("year");
        assert_eq!(named.name(), Some("year"));
        assert_eq!(named.labels(), idx.labels());
    }

    #[test]
    fn index_set_names_some_and_none() {
        let idx = Index::new(vec!["a".into(), "b".into()]);
        let named = idx.set_names(Some("letters"));
        assert_eq!(named.name(), Some("letters"));
        let cleared = named.set_names(None);
        assert_eq!(cleared.name(), None);
    }

    #[test]
    fn index_name_propagates_through_unique() {
        let idx = Index::new(vec![1_i64.into(), 1_i64.into(), 2_i64.into()]).set_name("id");
        let u = idx.unique();
        assert_eq!(u.name(), Some("id"));
        assert_eq!(u.len(), 2);
    }

    #[test]
    fn index_name_propagates_through_sort_values() {
        let idx = Index::new(vec![3_i64.into(), 1_i64.into(), 2_i64.into()]).set_name("val");
        let sorted = idx.sort_values();
        assert_eq!(sorted.name(), Some("val"));
    }

    #[test]
    fn index_name_propagates_through_take_and_slice() {
        let idx = Index::new(vec!["a".into(), "b".into(), "c".into()]).set_name("letter");
        assert_eq!(idx.take(&[0, 2]).name(), Some("letter"));
        assert_eq!(idx.slice(1, 2).name(), Some("letter"));
    }

    #[test]
    fn index_name_propagates_through_map() {
        let idx = Index::new(vec![1_i64.into(), 2_i64.into()]).set_name("x");
        let mapped = idx.map(|l| match l {
            IndexLabel::Int64(v) => IndexLabel::Int64(v * 10),
            other => other.clone(),
        });
        assert_eq!(mapped.name(), Some("x"));
    }

    #[test]
    fn index_name_propagates_through_drop_labels() {
        let idx = Index::new(vec![1_i64.into(), 2_i64.into(), 3_i64.into()]).set_name("num");
        let dropped = idx.drop_labels(&[2_i64.into()]);
        assert_eq!(dropped.name(), Some("num"));
        assert_eq!(dropped.len(), 2);
    }

    #[test]
    fn index_name_propagates_through_astype() {
        let idx = Index::new(vec![1_i64.into(), 2_i64.into()]).set_name("n");
        assert_eq!(idx.astype_str().name(), Some("n"));
        let idx2 = Index::new(vec!["1".into(), "2".into()]).set_name("s");
        assert_eq!(idx2.astype_int().name(), Some("s"));
    }

    #[test]
    fn index_name_shared_for_intersection() {
        let a = Index::new(vec![1_i64.into(), 2_i64.into()]).set_name("x");
        let b = Index::new(vec![2_i64.into(), 3_i64.into()]).set_name("x");
        assert_eq!(a.intersection(&b).name(), Some("x"));

        let c = Index::new(vec![2_i64.into(), 3_i64.into()]).set_name("y");
        assert_eq!(a.intersection(&c).name(), None);
    }

    #[test]
    fn index_name_shared_for_union() {
        let a = Index::new(vec![1_i64.into()]).set_name("k");
        let b = Index::new(vec![2_i64.into()]).set_name("k");
        assert_eq!(a.union_with(&b).name(), Some("k"));

        let c = Index::new(vec![2_i64.into()]);
        assert_eq!(a.union_with(&c).name(), None);
    }

    #[test]
    fn index_name_propagates_through_where_cond() {
        let idx = Index::new(vec!["a".into(), "b".into()]).set_name("col");
        let result = idx.where_cond(&[true, false], &"Z".into());
        assert_eq!(result.name(), Some("col"));
    }

    #[test]
    fn index_rename_index() {
        let idx = Index::new(vec![1_i64.into()]);
        let named = idx.rename_index(Some("foo"));
        assert_eq!(named.name(), Some("foo"));
        let cleared = named.rename_index(None);
        assert_eq!(cleared.name(), None);
    }

    #[test]
    fn index_equality_ignores_name() {
        let a = Index::new(vec![1_i64.into(), 2_i64.into()]).set_name("a");
        let b = Index::new(vec![1_i64.into(), 2_i64.into()]).set_name("b");
        assert_eq!(a, b);
    }

    #[test]
    fn index_names_property() {
        let idx = Index::new(vec![1_i64.into()]);
        assert_eq!(idx.names(), vec![None]);
        let named = idx.set_name("x");
        assert_eq!(named.names(), vec![Some("x".to_string())]);
    }

    #[test]
    fn index_set_names_list() {
        let idx = Index::new(vec![1_i64.into()]);
        let named = idx.set_names_list(&[Some("foo")]);
        assert_eq!(named.name(), Some("foo"));
        let cleared = named.set_names_list(&[None]);
        assert_eq!(cleared.name(), None);
    }

    #[test]
    fn index_to_flat_index() {
        let idx = Index::new(vec!["a".into(), "b".into()]).set_name("x");
        let flat = idx.to_flat_index();
        assert_eq!(flat, idx);
        assert_eq!(flat.name(), Some("x"));
    }

    // ── MultiIndex tests ──

    #[test]
    fn multi_index_from_tuples() {
        let mi = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["a".into(), 2_i64.into()],
            vec!["b".into(), 1_i64.into()],
        ])
        .unwrap();

        assert_eq!(mi.nlevels(), 2);
        assert_eq!(mi.len(), 3);
        assert!(!mi.is_empty());
    }

    #[test]
    fn multi_index_from_tuples_ragged_errors() {
        let err = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["b".into()], // wrong number of levels
        ]);
        assert!(err.is_err());
    }

    #[test]
    fn multi_index_from_arrays() {
        let mi = MultiIndex::from_arrays(vec![
            vec!["a".into(), "a".into(), "b".into()],
            vec![1_i64.into(), 2_i64.into(), 1_i64.into()],
        ])
        .unwrap();

        assert_eq!(mi.nlevels(), 2);
        assert_eq!(mi.len(), 3);
    }

    #[test]
    fn multi_index_from_arrays_length_mismatch_errors() {
        let err = MultiIndex::from_arrays(vec![
            vec!["a".into(), "b".into()],
            vec![1_i64.into()], // wrong length
        ]);
        assert!(err.is_err());
    }

    #[test]
    fn multi_index_from_frame_preserves_column_names_a1dv9() {
        let mi = MultiIndex::from_frame(vec![
            (
                Some("letter".into()),
                vec!["a".into(), "b".into(), "b".into()],
            ),
            (
                Some("number".into()),
                vec![1_i64.into(), 1_i64.into(), 2_i64.into()],
            ),
        ])
        .unwrap();

        assert_eq!(mi.names(), &[Some("letter".into()), Some("number".into())]);
        assert_eq!(
            mi.to_list(),
            vec![
                vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(1)],
                vec![IndexLabel::Utf8("b".into()), IndexLabel::Int64(1)],
                vec![IndexLabel::Utf8("b".into()), IndexLabel::Int64(2)],
            ]
        );

        let empty = MultiIndex::from_frame(Vec::new()).unwrap();
        assert!(empty.is_empty());
        assert_eq!(empty.nlevels(), 0);
    }

    #[test]
    fn multi_index_from_frame_rejects_length_mismatch_a1dv9() {
        let err = MultiIndex::from_frame(vec![
            (Some("letter".into()), vec!["a".into(), "b".into()]),
            (Some("number".into()), vec![1_i64.into()]),
        ])
        .unwrap_err();

        assert!(matches!(
            err,
            super::IndexError::LengthMismatch {
                expected: 2,
                actual: 1,
                ..
            }
        ));
    }

    #[test]
    fn multi_index_from_product() {
        let mi = MultiIndex::from_product(vec![
            vec!["a".into(), "b".into()],
            vec![1_i64.into(), 2_i64.into(), 3_i64.into()],
        ])
        .unwrap();

        assert_eq!(mi.nlevels(), 2);
        assert_eq!(mi.len(), 6); // 2 * 3
    }

    #[test]
    fn multi_index_from_product_values() {
        let mi = MultiIndex::from_product(vec![
            vec!["x".into(), "y".into()],
            vec![1_i64.into(), 2_i64.into()],
        ])
        .unwrap();

        // Should produce: (x,1), (x,2), (y,1), (y,2)
        assert_eq!(
            mi.get_tuple(0).unwrap(),
            vec![&IndexLabel::Utf8("x".into()), &IndexLabel::Int64(1)]
        );
        assert_eq!(
            mi.get_tuple(1).unwrap(),
            vec![&IndexLabel::Utf8("x".into()), &IndexLabel::Int64(2)]
        );
        assert_eq!(
            mi.get_tuple(2).unwrap(),
            vec![&IndexLabel::Utf8("y".into()), &IndexLabel::Int64(1)]
        );
        assert_eq!(
            mi.get_tuple(3).unwrap(),
            vec![&IndexLabel::Utf8("y".into()), &IndexLabel::Int64(2)]
        );
    }

    #[test]
    fn multi_index_get_level_values() {
        let mi = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
        ])
        .unwrap()
        .set_names(vec![Some("letter".into()), Some("number".into())]);

        let level0 = mi.get_level_values(0).unwrap();
        assert_eq!(
            level0.labels(),
            &[IndexLabel::Utf8("a".into()), IndexLabel::Utf8("b".into())]
        );
        assert_eq!(level0.name(), Some("letter"));

        let level1 = mi.get_level_values(1).unwrap();
        assert_eq!(
            level1.labels(),
            &[IndexLabel::Int64(1), IndexLabel::Int64(2)]
        );
        assert_eq!(level1.name(), Some("number"));
    }

    #[test]
    fn multi_index_get_level_values_out_of_bounds() {
        let mi = MultiIndex::from_tuples(vec![vec!["a".into()]]).unwrap();
        assert!(mi.get_level_values(1).is_err());
    }

    #[test]
    fn multi_index_metadata_shape_and_tuple_materialization() {
        let mi = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["a".into(), 2_i64.into()],
            vec!["b".into(), 1_i64.into()],
        ])
        .unwrap()
        .set_names(vec![Some("letter".into()), Some("number".into())]);

        let tuples = vec![
            vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(1)],
            vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(2)],
            vec![IndexLabel::Utf8("b".into()), IndexLabel::Int64(1)],
        ];
        assert_eq!(mi.name(), None);
        assert_eq!(mi.names(), &[Some("letter".into()), Some("number".into())]);
        assert_eq!(mi.size(), 3);
        assert_eq!(mi.shape(), (3,));
        assert_eq!(mi.ndim(), 1);
        assert!(!mi.empty());
        assert_eq!(mi.to_list(), tuples);
        assert_eq!(mi.tolist(), mi.to_list());
        assert_eq!(mi.to_numpy(), mi.to_list());
        assert_eq!(mi.values(), mi.to_list());
        assert_eq!(mi.array(), mi.to_list());
        assert_eq!(mi.ravel(), mi.to_list());
        assert_eq!(mi.format(), vec!["(a, 1)", "(a, 2)", "(b, 1)"]);
        assert_eq!(mi.view(), mi);
        assert_eq!(mi.transpose(), mi);
        assert_eq!(mi.T(), mi);
        assert_eq!(mi.to_frame(), tuples);
        assert_eq!(
            mi.to_series(),
            tuples
                .iter()
                .cloned()
                .map(|tuple| (tuple.clone(), tuple))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn multi_index_levels_codes_and_levshape_exclude_missing_labels() {
        let mi = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec![IndexLabel::Datetime64(i64::MIN), 2_i64.into()],
            vec!["a".into(), 1_i64.into()],
        ])
        .unwrap()
        .set_names(vec![Some("letter".into()), Some("number".into())]);

        let levels = mi.levels();
        assert_eq!(levels[0].labels(), &[IndexLabel::Utf8("a".into())]);
        assert_eq!(levels[0].name(), Some("letter"));
        assert_eq!(
            levels[1].labels(),
            &[IndexLabel::Int64(1), IndexLabel::Int64(2)]
        );
        assert_eq!(levels[1].name(), Some("number"));
        assert_eq!(mi.codes(), vec![vec![0, -1, 0], vec![0, 1, 0]]);
        assert_eq!(mi.levshape(), vec![1, 2]);
        assert!(mi.memory_usage(false) <= mi.memory_usage(true));
        assert_eq!(mi.nbytes(), mi.memory_usage(false));
    }

    #[test]
    fn multi_index_dtype_type_checks_and_item_match_object_index_shape() {
        let mi = MultiIndex::from_tuples(vec![vec!["a".into(), 1_i64.into()]])
            .unwrap()
            .set_names(vec![Some("letter".into()), Some("number".into())]);

        assert_eq!(mi.dtype(), "object");
        assert_eq!(mi.dtypes(), vec!["object", "int64"]);
        assert_eq!(mi.inferred_type(), "mixed");
        assert_eq!(mi.infer_objects(), mi);
        assert!(!mi.holds_integer());
        assert!(!mi.is_boolean());
        assert!(!mi.is_categorical());
        assert!(!mi.is_floating());
        assert!(!mi.is_integer());
        assert!(!mi.is_interval());
        assert!(!mi.is_numeric());
        assert!(mi.is_object());
        assert!(mi.is_(&mi));
        assert_eq!(
            mi.item().unwrap(),
            vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(1)]
        );

        let multi = mi.repeat(2);
        assert!(multi.item().is_err());
    }

    #[test]
    fn multi_index_missing_masks_fillna_putmask_where_and_map() {
        let mi = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec![IndexLabel::Datetime64(i64::MIN), 2_i64.into()],
            vec!["b".into(), IndexLabel::Timedelta64(Timedelta::NAT)],
            vec!["c".into(), 3_i64.into()],
        ])
        .unwrap()
        .set_names(vec![Some("letter".into()), Some("number".into())]);

        let missing_mask_errors = [
            mi.hasnans().unwrap_err(),
            mi.isna().unwrap_err(),
            mi.isnull().unwrap_err(),
            mi.notna().unwrap_err(),
            mi.notnull().unwrap_err(),
        ];
        for err in missing_mask_errors {
            assert!(matches!(
                err,
                super::IndexError::InvalidArgument(message)
                    if message == "isna is not defined for MultiIndex"
            ));
        }
        assert_eq!(mi.copy(), mi);
        assert_eq!(mi.remove_unused_levels(), mi);

        let scalar_filled = mi.fillna(&IndexLabel::Utf8("missing".into()));
        assert_eq!(
            scalar_filled.to_list(),
            vec![
                vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(1)],
                vec![IndexLabel::Utf8("missing".into()), IndexLabel::Int64(2)],
                vec![
                    IndexLabel::Utf8("b".into()),
                    IndexLabel::Utf8("missing".into())
                ],
                vec![IndexLabel::Utf8("c".into()), IndexLabel::Int64(3)],
            ]
        );

        let tuple_filled = mi
            .fillna_tuple(&[IndexLabel::Utf8("z".into()), IndexLabel::Int64(0)])
            .unwrap();
        assert_eq!(
            tuple_filled.to_list(),
            vec![
                vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(1)],
                vec![IndexLabel::Utf8("z".into()), IndexLabel::Int64(2)],
                vec![IndexLabel::Utf8("b".into()), IndexLabel::Int64(0)],
                vec![IndexLabel::Utf8("c".into()), IndexLabel::Int64(3)],
            ]
        );
        assert!(
            mi.fillna_tuple(&[IndexLabel::Utf8("short".into())])
                .is_err()
        );

        let masked = mi
            .putmask(
                &[false, true, false, true],
                vec![IndexLabel::Utf8("x".into()), IndexLabel::Int64(9)],
            )
            .unwrap();
        assert_eq!(
            masked.to_list(),
            vec![
                vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(1)],
                vec![IndexLabel::Utf8("x".into()), IndexLabel::Int64(9)],
                vec![
                    IndexLabel::Utf8("b".into()),
                    IndexLabel::Timedelta64(Timedelta::NAT)
                ],
                vec![IndexLabel::Utf8("x".into()), IndexLabel::Int64(9)],
            ]
        );
        assert!(
            mi.putmask(&[true], vec![IndexLabel::Utf8("x".into())])
                .is_err()
        );

        let where_result = mi
            .r#where(
                &[true, false, true, false],
                vec![IndexLabel::Utf8("fallback".into()), IndexLabel::Int64(5)],
            )
            .unwrap();
        assert_eq!(
            where_result.to_list(),
            vec![
                vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(1)],
                vec![IndexLabel::Utf8("fallback".into()), IndexLabel::Int64(5)],
                vec![
                    IndexLabel::Utf8("b".into()),
                    IndexLabel::Timedelta64(Timedelta::NAT)
                ],
                vec![IndexLabel::Utf8("fallback".into()), IndexLabel::Int64(5)],
            ]
        );

        let rendered = mi.map(|tuple| {
            tuple
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("|")
        });
        assert_eq!(rendered[0], "a|1");
        assert_eq!(rendered[3], "c|3");
    }

    #[test]
    fn multi_index_set_levels_and_set_codes_rebuild_from_pandas_catalogs() {
        let mi = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
            vec!["a".into(), 1_i64.into()],
        ])
        .unwrap()
        .set_names(vec![Some("letter".into()), Some("number".into())]);

        let relabeled = mi
            .set_levels(vec![
                vec![IndexLabel::Utf8("x".into()), IndexLabel::Utf8("y".into())],
                vec![IndexLabel::Int64(10), IndexLabel::Int64(20)],
            ])
            .unwrap();
        assert_eq!(
            relabeled.to_list(),
            vec![
                vec![IndexLabel::Utf8("x".into()), IndexLabel::Int64(10)],
                vec![IndexLabel::Utf8("y".into()), IndexLabel::Int64(20)],
                vec![IndexLabel::Utf8("x".into()), IndexLabel::Int64(10)],
            ]
        );
        assert_eq!(relabeled.names(), mi.names());
        assert!(
            mi.set_levels(vec![vec![IndexLabel::Utf8("only".into())]])
                .is_err()
        );
        assert!(
            mi.set_levels(vec![
                vec![IndexLabel::Utf8("x".into())],
                vec![IndexLabel::Int64(10), IndexLabel::Int64(20)],
            ])
            .is_err()
        );

        let recoded = mi.set_codes(vec![vec![1, 0, 1], vec![1, -1, 0]]).unwrap();
        assert_eq!(
            recoded.to_list(),
            vec![
                vec![IndexLabel::Utf8("b".into()), IndexLabel::Int64(2)],
                vec![
                    IndexLabel::Utf8("a".into()),
                    IndexLabel::Datetime64(i64::MIN)
                ],
                vec![IndexLabel::Utf8("b".into()), IndexLabel::Int64(1)],
            ]
        );
        assert_eq!(recoded.names(), mi.names());
        assert!(mi.set_codes(vec![vec![0, 1, 0]]).is_err());
        assert!(mi.set_codes(vec![vec![0, 1], vec![0, 1, 0]]).is_err());
        assert!(mi.set_codes(vec![vec![0, 1, 0], vec![0, 99, 0]]).is_err());
    }

    #[test]
    fn multi_index_equals_identical_and_equal_levels_match_pandas_names() {
        let left = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
        ])
        .unwrap()
        .set_names(vec![Some("letter".into()), Some("number".into())]);
        let renamed = left
            .clone()
            .set_names(vec![Some("letter".into()), Some("other".into())]);
        let reordered = MultiIndex::from_tuples(vec![
            vec!["b".into(), 2_i64.into()],
            vec!["a".into(), 1_i64.into()],
        ])
        .unwrap()
        .set_names(vec![Some("letter".into()), Some("number".into())]);

        assert!(left.equals(&renamed));
        assert!(!left.identical(&renamed));
        assert!(left.equal_levels(&renamed));
        assert!(!left.equals(&reordered));
        assert!(!left.equal_levels(&reordered));
    }

    #[test]
    fn multi_index_to_flat_index() {
        let mi = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
        ])
        .unwrap();

        let flat = mi.to_flat_index("_");
        assert_eq!(flat.labels()[0], IndexLabel::Utf8("a_1".into()));
        assert_eq!(flat.labels()[1], IndexLabel::Utf8("b_2".into()));
    }

    #[test]
    fn multi_index_droplevel() {
        let mi = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into(), "x".into()],
            vec!["b".into(), 2_i64.into(), "y".into()],
        ])
        .unwrap()
        .set_names(vec![
            Some("l0".into()),
            Some("l1".into()),
            Some("l2".into()),
        ]);

        // Drop middle level -> 2 levels remain -> MultiIndex
        let result = mi.droplevel(1).unwrap();
        assert!(
            matches!(&result, super::MultiIndexOrIndex::Multi(_)),
            "expected MultiIndex after dropping from 3 levels"
        );
        if let super::MultiIndexOrIndex::Multi(mi2) = result {
            assert_eq!(mi2.nlevels(), 2);
            assert_eq!(mi2.names(), &[Some("l0".into()), Some("l2".into())]);
        }
    }

    #[test]
    fn multi_index_droplevel_to_index() {
        let mi = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
        ])
        .unwrap()
        .set_names(vec![Some("letter".into()), Some("number".into())]);

        // Drop one level from 2 -> 1 level -> plain Index
        let result = mi.droplevel(0).unwrap();
        assert!(
            matches!(&result, super::MultiIndexOrIndex::Index(_)),
            "expected Index after dropping from 2 levels"
        );
        if let super::MultiIndexOrIndex::Index(idx) = result {
            assert_eq!(idx.labels(), &[IndexLabel::Int64(1), IndexLabel::Int64(2)]);
            assert_eq!(idx.name(), Some("number"));
        }
    }

    #[test]
    fn multi_index_swaplevel() {
        let mi = MultiIndex::from_tuples(vec![vec!["a".into(), 1_i64.into()]])
            .unwrap()
            .set_names(vec![Some("first".into()), Some("second".into())]);

        let swapped = mi.swaplevel(0, 1).unwrap();
        assert_eq!(
            swapped.names(),
            &[Some("second".into()), Some("first".into())]
        );
        assert_eq!(
            swapped.get_tuple(0).unwrap(),
            vec![&IndexLabel::Int64(1), &IndexLabel::Utf8("a".into())]
        );
    }

    #[test]
    fn multi_index_empty() {
        let mi = MultiIndex::from_tuples(vec![]).unwrap();
        assert_eq!(mi.nlevels(), 0);
        assert_eq!(mi.len(), 0);
        assert!(mi.is_empty());
    }

    #[test]
    fn multi_index_get_tuple_out_of_bounds() {
        let mi = MultiIndex::from_tuples(vec![vec!["a".into()]]).unwrap();
        assert!(mi.get_tuple(1).is_none());
    }

    #[test]
    fn multi_index_get_loc_tuple_exact_and_duplicates() {
        let mi = MultiIndex::from_arrays(vec![
            vec!["east".into(), "east".into(), "west".into(), "east".into()],
            vec!["A".into(), "B".into(), "A".into(), "A".into()],
        ])
        .unwrap();

        let positions = mi
            .get_loc_tuple(&[
                IndexLabel::Utf8("east".into()),
                IndexLabel::Utf8("A".into()),
            ])
            .unwrap();
        assert_eq!(positions, vec![0, 3]);
    }

    #[test]
    fn multi_index_get_loc_level_prefix_returns_remaining_index() {
        let mi = MultiIndex::from_arrays(vec![
            vec!["east".into(), "east".into(), "west".into()],
            vec!["A".into(), "B".into(), "A".into()],
        ])
        .unwrap()
        .set_names(vec![Some("region".into()), Some("product".into())]);

        let (positions, remaining) = mi
            .get_loc_level(&[IndexLabel::Utf8("east".into())])
            .unwrap();
        assert_eq!(positions, vec![0, 1]);
        assert!(matches!(
            &remaining,
            Some(super::MultiIndexOrIndex::Index(index))
                if index.labels()
                    == [IndexLabel::Utf8("A".into()), IndexLabel::Utf8("B".into())]
                    && index.name() == Some("product")
        ));
    }

    #[test]
    fn multi_index_groupby_join_groups_duplicate_tuples_d89fe3() {
        let mi = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
            vec!["a".into(), 1_i64.into()],
        ])
        .unwrap();

        let groups = mi.groupby();
        assert_eq!(
            groups[&vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(1)]],
            vec![0, 2]
        );
        assert_eq!(
            groups[&vec![IndexLabel::Utf8("b".into()), IndexLabel::Int64(2)]],
            vec![1]
        );
    }

    #[test]
    fn multi_index_groupby_join_modes_d89fe3() {
        let left = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
            vec!["c".into(), 3_i64.into()],
        ])
        .unwrap();
        let right = MultiIndex::from_tuples(vec![
            vec!["b".into(), 2_i64.into()],
            vec!["d".into(), 4_i64.into()],
        ])
        .unwrap();

        assert_eq!(left.join(&right, "left").unwrap(), left);
        assert_eq!(left.join(&right, "right").unwrap(), right);
        assert_eq!(
            left.join(&right, "inner").unwrap().to_list(),
            vec![vec!["b".into(), 2_i64.into()]]
        );
        assert_eq!(
            left.join(&right, "outer").unwrap().to_list(),
            vec![
                vec!["a".into(), 1_i64.into()],
                vec!["b".into(), 2_i64.into()],
                vec!["c".into(), 3_i64.into()],
                vec!["d".into(), 4_i64.into()]
            ]
        );
    }

    #[test]
    fn multi_index_groupby_join_rejects_bad_mode_and_level_mismatch_d89fe3() {
        let left = MultiIndex::from_tuples(vec![vec!["a".into(), 1_i64.into()]]).unwrap();
        let right = MultiIndex::from_tuples(vec![vec!["a".into()]]).unwrap();

        assert!(left.join(&right, "sideways").is_err());
        assert!(left.join(&right, "inner").is_err());
        assert!(left.join(&right, "outer").is_err());
    }

    #[test]
    fn multi_index_slice_locs_uses_lexicographic_bounds() {
        let mi = MultiIndex::from_arrays(vec![
            vec!["east".into(), "east".into(), "west".into(), "west".into()],
            vec![1_i64.into(), 2_i64.into(), 1_i64.into(), 2_i64.into()],
        ])
        .unwrap();

        let (start, stop) = mi
            .slice_locs(
                Some(&[IndexLabel::Utf8("east".into()), IndexLabel::Int64(2)]),
                Some(&[IndexLabel::Utf8("west".into()), IndexLabel::Int64(1)]),
            )
            .unwrap();
        assert_eq!((start, stop), (1, 3));
    }

    #[test]
    fn multi_index_slice_bound_partial_prefixes_d89fe2() {
        let mi = MultiIndex::from_arrays(vec![
            vec!["east".into(), "east".into(), "west".into(), "west".into()],
            vec![1_i64.into(), 2_i64.into(), 1_i64.into(), 2_i64.into()],
        ])
        .unwrap();

        let east = [IndexLabel::Utf8("east".into())];
        assert_eq!(mi.get_slice_bound(&east, "left").unwrap(), 0);
        assert_eq!(mi.get_slice_bound(&east, "right").unwrap(), 2);

        let west = [IndexLabel::Utf8("west".into())];
        assert_eq!(mi.slice_indexer(Some(&west), None).unwrap(), (2, 4));
        assert_eq!(mi.slice_indexer(None, Some(&east)).unwrap(), (0, 2));
    }

    #[test]
    fn multi_index_slice_bound_full_tuple_and_missing_insert_d89fe2() {
        let mi = MultiIndex::from_arrays(vec![
            vec!["east".into(), "east".into(), "west".into(), "west".into()],
            vec![1_i64.into(), 2_i64.into(), 1_i64.into(), 2_i64.into()],
        ])
        .unwrap();

        let exact = [IndexLabel::Utf8("east".into()), IndexLabel::Int64(2)];
        assert_eq!(mi.get_slice_bound(&exact, "left").unwrap(), 1);
        assert_eq!(mi.get_slice_bound(&exact, "right").unwrap(), 2);

        let missing_insert = [IndexLabel::Utf8("east".into()), IndexLabel::Int64(3)];
        assert_eq!(mi.get_slice_bound(&missing_insert, "left").unwrap(), 2);
        assert_eq!(mi.get_slice_bound(&missing_insert, "right").unwrap(), 2);
    }

    #[test]
    fn multi_index_slice_bound_rejects_invalid_side_d89fe2() {
        let mi = MultiIndex::from_tuples(vec![vec![IndexLabel::Utf8("east".into())]]).unwrap();
        let key = [IndexLabel::Utf8("east".into())];

        assert!(mi.get_slice_bound(&key, "middle").is_err());
    }

    #[test]
    fn multi_index_truncate_uses_prefix_bounds_d89fe11() -> Result<(), super::IndexError> {
        let mi = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["a".into(), 3_i64.into()],
            vec!["b".into(), 1_i64.into()],
            vec!["c".into(), 1_i64.into()],
        ])?
        .set_names(vec![Some("letter".into()), Some("number".into())]);

        let bounded = mi.truncate(
            Some(&[IndexLabel::Utf8("a".into())]),
            Some(&[IndexLabel::Utf8("b".into())]),
        )?;
        assert_eq!(
            bounded.to_list(),
            vec![
                vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(1)],
                vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(3)],
                vec![IndexLabel::Utf8("b".into()), IndexLabel::Int64(1)],
            ]
        );
        assert_eq!(bounded.names(), mi.names());

        let tail = mi.truncate(Some(&[IndexLabel::Utf8("b".into())]), None)?;
        assert_eq!(
            tail.to_list(),
            vec![
                vec![IndexLabel::Utf8("b".into()), IndexLabel::Int64(1)],
                vec![IndexLabel::Utf8("c".into()), IndexLabel::Int64(1)],
            ]
        );

        let clipped = mi.truncate(None, Some(&[IndexLabel::Utf8("aa".into())]))?;
        assert_eq!(
            clipped.to_list(),
            vec![
                vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(1)],
                vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(3)],
            ]
        );

        let empty = mi.truncate(Some(&[IndexLabel::Utf8("d".into())]), None)?;
        assert!(empty.is_empty());
        assert_eq!(empty.names(), mi.names());

        Ok(())
    }

    #[test]
    fn multi_index_get_locs_prefix_and_exact_selectors_d89fe10() -> Result<(), super::IndexError> {
        let mi = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["a".into(), 2_i64.into()],
            vec!["b".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
        ])?;

        assert_eq!(mi.get_locs(&[IndexLabel::Utf8("a".into())])?, vec![0, 1]);
        assert_eq!(
            mi.get_locs(&[IndexLabel::Utf8("a".into()), IndexLabel::Int64(1)])?,
            vec![0]
        );
        assert_eq!(mi.get_locs(&[])?, Vec::<usize>::new());

        Ok(())
    }

    #[test]
    fn multi_index_get_locs_rejects_missing_and_overlong_keys_d89fe10()
    -> Result<(), super::IndexError> {
        let mi = MultiIndex::from_tuples(vec![vec!["a".into(), 1_i64.into()]])?;

        assert!(mi.get_locs(&[IndexLabel::Utf8("z".into())]).is_err());
        assert!(
            mi.get_locs(&[
                IndexLabel::Utf8("a".into()),
                IndexLabel::Int64(1),
                IndexLabel::Utf8("extra".into()),
            ])
            .is_err()
        );

        Ok(())
    }

    #[test]
    fn multi_index_get_indexer_non_unique_expands_duplicate_matches() {
        let source = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["a".into(), 2_i64.into()],
            vec!["b".into(), 1_i64.into()],
            vec!["a".into(), 1_i64.into()],
        ])
        .unwrap();
        let target = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["z".into(), 9_i64.into()],
            vec!["a".into(), 2_i64.into()],
            vec!["a".into(), 1_i64.into()],
        ])
        .unwrap();

        let (indexer, missing) = source.get_indexer_non_unique(&target);
        assert_eq!(indexer, vec![0, 3, -1, 1, 0, 3]);
        assert_eq!(missing, vec![1]);
    }

    #[test]
    fn multi_index_setop_packed_matches_reference_misetop() {
        // intersection/difference packed path must equal an independent
        // tuple-set reference (mixed Utf8+Int64 levels, duplicate self rows,
        // partial overlap, disjoint, and empty other).
        let mk = |spec: &[(&str, i64)]| {
            MultiIndex::from_tuples(
                spec.iter()
                    .map(|(s, i)| vec![IndexLabel::Utf8((*s).to_string()), IndexLabel::Int64(*i)])
                    .collect::<Vec<_>>(),
            )
            .unwrap()
        };
        type TupleSpec = Vec<(&'static str, i64)>;
        type SetopCase = (TupleSpec, TupleSpec);

        let cases: Vec<SetopCase> = vec![
            (
                vec![("a", 1), ("b", 2), ("a", 1), ("c", 3), ("b", 2)],
                vec![("b", 2), ("c", 3), ("z", 9)],
            ),
            (vec![("a", 1), ("b", 2)], vec![("x", 7), ("y", 8)]),
            (vec![("a", 1), ("a", 1), ("b", 2)], vec![("a", 1)]),
        ];
        for (sa, sb) in cases {
            let a = mk(&sa);
            let b = mk(&sb);
            let bset: std::collections::HashSet<Vec<IndexLabel>> =
                b.to_list().into_iter().collect();

            let mut seen = std::collections::HashSet::new();
            let ref_inter: Vec<Vec<IndexLabel>> = a
                .to_list()
                .into_iter()
                .filter(|t| bset.contains(t) && seen.insert(t.clone()))
                .collect();
            assert_eq!(
                a.intersection(&b).unwrap().to_list(),
                ref_inter,
                "inter {sa:?}"
            );

            let mut seen_d = std::collections::HashSet::new();
            let ref_diff: Vec<Vec<IndexLabel>> = a
                .to_list()
                .into_iter()
                .filter(|t| !bset.contains(t) && seen_d.insert(t.clone()))
                .collect();
            assert_eq!(a.difference(&b).unwrap().to_list(), ref_diff, "diff {sa:?}");
        }
    }

    #[test]
    fn multi_index_duplicated_packed_matches_vec_reference_midedup() {
        // The identity-packed-key duplicated path must equal an independent
        // Vec<IndexLabel>-key reference for all keep modes (mixed Utf8+Int64
        // levels with duplicate tuples).
        let n = 400usize;
        let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut l0 = Vec::with_capacity(n);
        let mut l1 = Vec::with_capacity(n);
        for _ in 0..n {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            l0.push(IndexLabel::Utf8(format!("g{}", (state >> 40) % 6)));
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            l1.push(IndexLabel::Int64(((state >> 40) % 5) as i64));
        }
        let mi = MultiIndex::from_arrays(vec![l0, l1]).unwrap();
        let rows = mi.to_list();

        for keep in [
            DuplicateKeep::First,
            DuplicateKeep::Last,
            DuplicateKeep::None,
        ] {
            let mut want = vec![false; n];
            match keep {
                DuplicateKeep::First => {
                    let mut seen = std::collections::HashSet::new();
                    for (r, w) in want.iter_mut().enumerate() {
                        if !seen.insert(rows[r].clone()) {
                            *w = true;
                        }
                    }
                }
                DuplicateKeep::Last => {
                    let mut seen = std::collections::HashSet::new();
                    for r in (0..n).rev() {
                        if !seen.insert(rows[r].clone()) {
                            want[r] = true;
                        }
                    }
                }
                DuplicateKeep::None => {
                    let mut counts: std::collections::HashMap<Vec<IndexLabel>, usize> =
                        Default::default();
                    for r in &rows {
                        *counts.entry(r.clone()).or_insert(0) += 1;
                    }
                    for (r, w) in want.iter_mut().enumerate() {
                        if counts[&rows[r]] > 1 {
                            *w = true;
                        }
                    }
                }
            }
            assert_eq!(mi.duplicated(keep), want, "duplicated {keep:?}");
        }
        // drop_duplicates/unique derive from duplicated(First).
        let mut seen = std::collections::HashSet::new();
        let kept: Vec<Vec<IndexLabel>> = rows
            .iter()
            .filter(|r| seen.insert((*r).clone()))
            .cloned()
            .collect();
        assert_eq!(mi.unique().to_list(), kept);
        assert_eq!(mi.nunique(), kept.len());
    }

    #[test]
    fn multi_index_argsort_packed_matches_tuple_sort_misort() {
        // The sorted-packed-key argsort must equal the level-by-level tuple
        // comparison sort (stable, original-position tiebreak) for mixed
        // Utf8+Int64 levels with duplicate tuples and shuffled order.
        let n = 600usize;
        let mut state: u64 = 0x1234_5678_9abc_def1;
        let mut l0 = Vec::with_capacity(n);
        let mut l1 = Vec::with_capacity(n);
        for _ in 0..n {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let a = (state >> 33) % 7; // low cardinality -> duplicate tuples
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let b = (state >> 33) % 5;
            l0.push(IndexLabel::Utf8(format!("g{a}")));
            l1.push(IndexLabel::Int64(b as i64));
        }
        let mi = MultiIndex::from_arrays(vec![l0, l1]).unwrap();

        // Independent reference: stable sort by lexicographic tuple, ties by pos.
        let rows = mi.to_list();
        let mut want: Vec<usize> = (0..n).collect();
        want.sort_by(|&a, &b| rows[a].cmp(&rows[b]).then(a.cmp(&b)));

        assert_eq!(mi.argsort(), want, "argsort");
        assert_eq!(
            mi.sort_values().to_list(),
            mi.take_existing_positions(&want).to_list()
        );
        // min/max derive from argsort and must match the reference ends.
        assert_eq!(mi.min(), Some(rows[want[0]].clone()));
        assert_eq!(mi.max(), Some(rows[want[n - 1]].clone()));
    }

    #[test]
    fn multi_index_get_indexer_packed_matches_vec_reference_mipack() {
        // The packed-u64-key path must equal an independent Vec<IndexLabel>-key
        // reference (mixed Utf8+Int64 levels, duplicate source, target-only
        // values exercising fresh per-level codes and the mixed-radix packing).
        let mk = |spec: &[(&str, i64)]| {
            MultiIndex::from_tuples(
                spec.iter()
                    .map(|(s, i)| vec![IndexLabel::Utf8((*s).to_string()), IndexLabel::Int64(*i)])
                    .collect::<Vec<_>>(),
            )
            .unwrap()
        };
        let source = mk(&[("a", 1), ("a", 2), ("b", 1), ("a", 1), ("c", 5), ("b", 2)]);
        let target = mk(&[
            ("b", 1),
            ("z", 9),
            ("a", 1),
            ("a", 2),
            ("c", 5),
            ("q", 0),
            ("b", 2),
        ]);
        let src_rows = source.to_list();
        let tgt_rows = target.to_list();

        let mut pos: std::collections::HashMap<Vec<IndexLabel>, Vec<usize>> = Default::default();
        for (r, key) in src_rows.iter().enumerate() {
            pos.entry(key.clone()).or_default().push(r);
        }
        let mut ref_ix = Vec::new();
        let mut ref_miss = Vec::new();
        for (tr, key) in tgt_rows.iter().enumerate() {
            if let Some(m) = pos.get(key) {
                ref_ix.extend(m.iter().map(|&p| p as isize));
            } else {
                ref_ix.push(-1);
                ref_miss.push(tr);
            }
        }
        let (ix, miss) = source.get_indexer_non_unique(&target);
        assert_eq!(ix, ref_ix, "non_unique indexer");
        assert_eq!(miss, ref_miss, "non_unique missing");

        let usrc = mk(&[("a", 1), ("a", 2), ("b", 1), ("c", 5), ("b", 2)]);
        let urows = usrc.to_list();
        let mut upos: std::collections::HashMap<Vec<IndexLabel>, isize> = Default::default();
        for (r, key) in urows.iter().enumerate() {
            upos.entry(key.clone()).or_insert(r as isize);
        }
        let ref_u: Vec<isize> = tgt_rows
            .iter()
            .map(|k| upos.get(k).copied().unwrap_or(-1))
            .collect();
        assert_eq!(usrc.get_indexer(&target).unwrap(), ref_u, "unique indexer");
    }

    #[test]
    fn multi_index_get_indexer_unique_maps_hits_and_missing_d89fe1() -> Result<(), super::IndexError>
    {
        let source = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
            vec!["c".into(), 3_i64.into()],
        ])?;
        let target = MultiIndex::from_tuples(vec![
            vec!["b".into(), 2_i64.into()],
            vec!["z".into(), 9_i64.into()],
            vec!["a".into(), 1_i64.into()],
        ])?;

        assert_eq!(source.get_indexer(&target)?, vec![1, -1, 0]);
        assert_eq!(source.get_indexer_for(&target)?, vec![1, -1, 0]);

        Ok(())
    }

    #[test]
    fn multi_index_get_indexer_rejects_duplicate_source_d89fe1() -> Result<(), super::IndexError> {
        let source = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
        ])?;
        let target = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
        ])?;

        let err = match source.get_indexer(&target) {
            Ok(indexer) => {
                return Err(super::IndexError::InvalidArgument(format!(
                    "duplicate source index unexpectedly returned {indexer:?}"
                )));
            }
            Err(err) => err,
        };
        assert!(matches!(
            err,
            super::IndexError::InvalidArgument(message)
                if message == "get_indexer requires a uniquely valued MultiIndex"
        ));
        assert_eq!(source.get_indexer_for(&target)?, vec![0, 1, 2]);

        Ok(())
    }

    #[test]
    fn multi_index_get_indexer_level_mismatch_marks_missing_d89fe1() -> Result<(), super::IndexError>
    {
        let source = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
        ])?;
        let target = MultiIndex::from_tuples(vec![vec!["a".into()], vec!["b".into()]])?;

        assert_eq!(source.get_indexer(&target)?, vec![-1, -1]);
        assert_eq!(source.get_indexer_for(&target)?, vec![-1, -1]);

        Ok(())
    }

    #[test]
    fn multi_index_reindex_maps_target_hits_and_missing_d89fe4() -> Result<(), super::IndexError> {
        let source = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
            vec!["c".into(), 3_i64.into()],
        ])?;
        let target = MultiIndex::from_tuples(vec![
            vec!["b".into(), 2_i64.into()],
            vec!["z".into(), 9_i64.into()],
            vec!["a".into(), 1_i64.into()],
        ])?
        .set_names(vec![Some("letter".into()), Some("number".into())]);

        let (reindexed, indexer) = source.reindex(&target)?;
        assert_eq!(reindexed, target);
        assert_eq!(indexer, vec![1, -1, 0]);

        Ok(())
    }

    #[test]
    fn multi_index_reindex_rejects_duplicate_source_d89fe4() -> Result<(), super::IndexError> {
        let source = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["a".into(), 1_i64.into()],
        ])?;
        let target = MultiIndex::from_tuples(vec![vec!["a".into(), 1_i64.into()]])?;

        assert!(source.reindex(&target).is_err());

        Ok(())
    }

    #[test]
    fn multi_index_reindex_level_mismatch_marks_missing_d89fe4() -> Result<(), super::IndexError> {
        let source = MultiIndex::from_tuples(vec![vec!["a".into(), 1_i64.into()]])?;
        let target = MultiIndex::from_tuples(vec![vec!["a".into()]])?;

        let (reindexed, indexer) = source.reindex(&target)?;
        assert_eq!(reindexed, target);
        assert_eq!(indexer, vec![-1]);

        Ok(())
    }

    #[test]
    fn multi_index_rename_replaces_all_names_d89fe5() -> Result<(), super::IndexError> {
        let source = MultiIndex::from_tuples(vec![vec!["a".into(), 1_i64.into()]])?
            .set_names(vec![Some("old0".into()), Some("old1".into())]);

        let renamed = source.rename(vec![Some("new0".into()), Some("new1".into())])?;

        assert_eq!(renamed.names(), &[Some("new0".into()), Some("new1".into())]);
        assert_eq!(source.names(), &[Some("old0".into()), Some("old1".into())]);
        assert_eq!(renamed.to_list(), source.to_list());

        Ok(())
    }

    #[test]
    fn multi_index_rename_level_replaces_one_name_d89fe5() -> Result<(), super::IndexError> {
        let source = MultiIndex::from_tuples(vec![vec!["a".into(), 1_i64.into()]])?
            .set_names(vec![Some("old0".into()), Some("old1".into())]);

        let renamed = source.rename_level(Some("new1".into()), 1)?;

        assert_eq!(renamed.names(), &[Some("old0".into()), Some("new1".into())]);
        assert_eq!(source.names(), &[Some("old0".into()), Some("old1".into())]);

        Ok(())
    }

    #[test]
    fn multi_index_rename_rejects_wrong_name_count_d89fe5() -> Result<(), super::IndexError> {
        let source = MultiIndex::from_tuples(vec![vec!["a".into(), 1_i64.into()]])?;

        let err = source.rename(vec![Some("only".into())]).unwrap_err();

        assert!(matches!(
            err,
            super::IndexError::LengthMismatch {
                expected: 2,
                actual: 1,
                ..
            }
        ));

        Ok(())
    }

    #[test]
    fn multi_index_rename_level_rejects_out_of_bounds_d89fe5() -> Result<(), super::IndexError> {
        let source = MultiIndex::from_tuples(vec![vec!["a".into(), 1_i64.into()]])?;

        let err = source.rename_level(Some("missing".into()), 2).unwrap_err();

        assert!(matches!(
            err,
            super::IndexError::OutOfBounds {
                position: 2,
                length: 2
            }
        ));

        Ok(())
    }

    #[test]
    fn multi_index_searchsorted_left_and_right_d89fe6() -> Result<(), super::IndexError> {
        let source = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["a".into(), 3_i64.into()],
            vec!["b".into(), 2_i64.into()],
            vec!["b".into(), 2_i64.into()],
        ])?;
        let target = MultiIndex::from_tuples(vec![
            vec!["a".into(), 0_i64.into()],
            vec!["a".into(), 1_i64.into()],
            vec!["a".into(), 2_i64.into()],
            vec!["a".into(), 3_i64.into()],
            vec!["b".into(), 2_i64.into()],
            vec!["c".into(), 0_i64.into()],
        ])?;

        assert_eq!(
            source.searchsorted(&target, "left")?,
            vec![0, 0, 1, 1, 2, 4]
        );
        assert_eq!(
            source.searchsorted(&target, "right")?,
            vec![0, 1, 1, 2, 4, 4]
        );

        Ok(())
    }

    #[test]
    fn multi_index_searchsorted_empty_target_d89fe6() -> Result<(), super::IndexError> {
        let source = MultiIndex::from_tuples(vec![vec!["a".into(), 1_i64.into()]])?;
        let target = MultiIndex::from_tuples(Vec::new())?;

        assert_eq!(source.searchsorted(&target, "left")?, Vec::<usize>::new());

        Ok(())
    }

    #[test]
    fn multi_index_searchsorted_rejects_invalid_side_d89fe6() -> Result<(), super::IndexError> {
        let source = MultiIndex::from_tuples(vec![vec!["a".into(), 1_i64.into()]])?;
        let target = MultiIndex::from_tuples(vec![vec!["a".into(), 1_i64.into()]])?;

        let err = source.searchsorted(&target, "middle").unwrap_err();

        assert!(matches!(
            err,
            super::IndexError::InvalidArgument(message)
                if message == "searchsorted: side must be 'left' or 'right', got \"middle\""
        ));

        Ok(())
    }

    #[test]
    fn multi_index_get_indexer_non_unique_level_mismatch_marks_all_missing() {
        let source = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
        ])
        .unwrap();
        let target = MultiIndex::from_tuples(vec![vec!["a".into()], vec!["b".into()]]).unwrap();

        let (indexer, missing) = source.get_indexer_non_unique(&target);
        assert_eq!(indexer, vec![-1, -1]);
        assert_eq!(missing, vec![0, 1]);
    }

    #[test]
    fn multi_index_isin_tuple_membership() {
        let mi = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
            vec!["a".into(), 3_i64.into()],
        ])
        .unwrap();
        let needles: Vec<Vec<IndexLabel>> = vec![
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
        ];
        assert_eq!(mi.isin(&needles), vec![true, true, false]);
    }

    #[test]
    fn multi_index_isin_ignores_mismatched_tuple_length() {
        let mi = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
        ])
        .unwrap();
        // Wrong-arity tuple contributes no matches.
        let needles: Vec<Vec<IndexLabel>> = vec![vec!["a".into()]];
        assert_eq!(mi.isin(&needles), vec![false, false]);
    }

    #[test]
    fn multi_index_isin_empty_values_yields_all_false() {
        let mi = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
        ])
        .unwrap();
        let needles: Vec<Vec<IndexLabel>> = Vec::new();
        assert_eq!(mi.isin(&needles), vec![false, false]);
    }

    #[test]
    fn multi_index_isin_level_filters_by_level() {
        let mi = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
            vec!["a".into(), 3_i64.into()],
        ])
        .unwrap();
        let level0 = mi.isin_level(&["a".into()], 0).unwrap();
        assert_eq!(level0, vec![true, false, true]);
        let level1 = mi.isin_level(&[2_i64.into(), 3_i64.into()], 1).unwrap();
        assert_eq!(level1, vec![false, true, true]);
    }

    #[test]
    fn multi_index_isin_level_out_of_bounds_errors() {
        let mi = MultiIndex::from_tuples(vec![vec!["a".into(), 1_i64.into()]]).unwrap();
        let err = mi.isin_level(&["a".into()], 5).unwrap_err();
        assert!(matches!(err, crate::IndexError::OutOfBounds { .. }));
    }

    #[test]
    fn multi_index_isin_empty_index_yields_empty() {
        let mi = MultiIndex::from_tuples(Vec::new()).unwrap();
        let needles: Vec<Vec<IndexLabel>> = vec![vec!["a".into(), 1_i64.into()]];
        assert_eq!(mi.isin(&needles), Vec::<bool>::new());
    }

    #[test]
    fn multi_index_duplicated_keep_first_default() {
        let mi = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
            vec!["a".into(), 1_i64.into()],
            vec!["c".into(), 3_i64.into()],
        ])
        .unwrap();
        let dup = mi.duplicated(DuplicateKeep::First);
        assert_eq!(dup, vec![false, false, true, false]);
    }

    #[test]
    fn multi_index_duplicated_keep_last_marks_earlier_occurrences() {
        let mi = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
        ])
        .unwrap();
        let dup = mi.duplicated(DuplicateKeep::Last);
        assert_eq!(dup, vec![true, false, false]);
    }

    #[test]
    fn multi_index_duplicated_keep_none_marks_all_repeats() {
        let mi = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
            vec!["a".into(), 1_i64.into()],
            vec!["c".into(), 3_i64.into()],
        ])
        .unwrap();
        let dup = mi.duplicated(DuplicateKeep::None);
        assert_eq!(dup, vec![true, false, true, false]);
    }

    #[test]
    fn multi_index_is_unique_true_and_false() {
        let unique = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
        ])
        .unwrap();
        assert!(unique.is_unique());
        assert!(!unique.has_duplicates());

        let duped = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["a".into(), 1_i64.into()],
        ])
        .unwrap();
        assert!(!duped.is_unique());
        assert!(duped.has_duplicates());
    }

    #[test]
    fn multi_index_duplicated_empty_yields_empty() {
        let mi = MultiIndex::from_tuples(Vec::new()).unwrap();
        assert_eq!(mi.duplicated(DuplicateKeep::First), Vec::<bool>::new());
        assert!(mi.is_unique());
    }

    #[test]
    fn multi_index_all_any_reject_bool_reduction_d89fe7() -> Result<(), super::IndexError> {
        let non_empty = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
        ])?;
        let empty = MultiIndex::from_arrays(vec![Vec::new(), Vec::new()])?;

        let cases = [
            (
                non_empty.all().unwrap_err(),
                "cannot perform all with this index type: MultiIndex",
            ),
            (
                non_empty.any().unwrap_err(),
                "cannot perform any with this index type: MultiIndex",
            ),
            (
                empty.all().unwrap_err(),
                "cannot perform all with this index type: MultiIndex",
            ),
            (
                empty.any().unwrap_err(),
                "cannot perform any with this index type: MultiIndex",
            ),
        ];

        for (err, expected) in cases {
            assert!(matches!(
                err,
                super::IndexError::InvalidArgument(message) if message == expected
            ));
        }

        Ok(())
    }

    #[test]
    fn multi_index_shift_rejects_temporal_shift_d89fe9() -> Result<(), super::IndexError> {
        let mi = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
        ])?;
        let expected = "This method is only implemented for DatetimeIndex, PeriodIndex and TimedeltaIndex; Got type MultiIndex";

        for err in [
            mi.shift(1, None).unwrap_err(),
            mi.shift(0, None).unwrap_err(),
            mi.shift(1, Some("D")).unwrap_err(),
        ] {
            assert!(matches!(
                err,
                super::IndexError::InvalidArgument(message) if message == expected
            ));
        }

        Ok(())
    }

    #[test]
    fn multi_index_str_rejects_string_accessor_d89fe12() -> Result<(), super::IndexError> {
        let mi = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
        ])?;

        let err = mi.r#str().unwrap_err();

        assert!(matches!(
            err,
            super::IndexError::InvalidArgument(message)
                if message == "Can only use .str accessor with Index, not MultiIndex"
        ));

        Ok(())
    }

    #[test]
    fn multi_index_astype_object_clones_other_dtypes_reject_c2x17() -> Result<(), super::IndexError>
    {
        let mi = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
        ])?;

        for dtype in ["object", "O"] {
            let cloned = mi.astype(dtype)?;
            assert!(cloned.equals(&mi));
            assert_eq!(cloned.nlevels(), mi.nlevels());
            assert_eq!(cloned.len(), mi.len());
        }

        let cat_err = mi.astype("category").unwrap_err();
        assert!(matches!(
            cat_err,
            super::IndexError::InvalidArgument(message)
                if message == "> 1 ndim Categorical are not supported at this time"
        ));

        for dtype in ["int64", "float64", "datetime64[ns]"] {
            let err = mi.astype(dtype).unwrap_err();
            let expected = format!(
                "Setting a MultiIndex dtype to anything other than object is not supported; got {dtype}"
            );
            assert!(matches!(
                err,
                super::IndexError::InvalidArgument(message) if message == expected
            ));
        }

        Ok(())
    }

    #[test]
    fn multi_index_diff_rejects_tuple_subtraction_c2x17() -> Result<(), super::IndexError> {
        let mi = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
            vec!["c".into(), 3_i64.into()],
        ])?;
        let expected = "cannot perform __sub__ with this index type: MultiIndex";

        for periods in [-1_i64, 0, 1, 2] {
            let err = mi.diff(periods).unwrap_err();
            assert!(matches!(
                err,
                super::IndexError::InvalidArgument(message) if message == expected
            ));
        }

        Ok(())
    }

    #[test]
    fn multi_index_round_rejects_tuple_rint_c2x17() -> Result<(), super::IndexError> {
        let mi = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
        ])?;
        let expected = "loop of ufunc does not support argument 0 of type tuple which has no callable rint method";

        for decimals in [-1_i32, 0, 1, 4] {
            let err = mi.round(decimals).unwrap_err();
            assert!(matches!(
                err,
                super::IndexError::InvalidArgument(message) if message == expected
            ));
        }

        Ok(())
    }

    #[test]
    fn range_index_argmax_argmin_handles_step_direction_mrchb() {
        let asc = super::RangeIndex::new(0, 5, 1).unwrap();
        assert_eq!(asc.argmax().unwrap(), 4);
        assert_eq!(asc.argmin().unwrap(), 0);

        let desc = super::RangeIndex::new(10, 0, -2).unwrap();
        assert_eq!(desc.argmax().unwrap(), 0);
        assert_eq!(desc.argmin().unwrap(), desc.len() - 1);

        let big_step = super::RangeIndex::new(1, 100, 7).unwrap();
        assert_eq!(big_step.argmax().unwrap(), big_step.len() - 1);
        assert_eq!(big_step.argmin().unwrap(), 0);
    }

    #[test]
    fn range_index_argmax_argmin_reject_empty_mrchb() {
        let empty = super::RangeIndex::new(5, 5, 1).unwrap();
        assert!(empty.is_empty());
        let max_err = empty.argmax().unwrap_err();
        assert!(matches!(
            max_err,
            super::IndexError::InvalidArgument(ref message)
                if message == "attempt to get argmax of an empty sequence"
        ));
        let min_err = empty.argmin().unwrap_err();
        assert!(matches!(
            min_err,
            super::IndexError::InvalidArgument(ref message)
                if message == "attempt to get argmin of an empty sequence"
        ));
    }

    #[test]
    fn range_index_argsort_orientation_matches_step_sign_mrchb() {
        let asc = super::RangeIndex::new(0, 5, 1).unwrap();
        assert_eq!(asc.argsort(), vec![0, 1, 2, 3, 4]);

        let desc = super::RangeIndex::new(10, 0, -2).unwrap();
        assert_eq!(desc.argsort(), vec![4, 3, 2, 1, 0]);

        let empty = super::RangeIndex::new(0, 0, 1).unwrap();
        assert_eq!(empty.argsort(), Vec::<usize>::new());
    }

    #[test]
    fn datetime_index_time_of_day_accessors_match_pandas_znejf() {
        // 2024-01-01T12:34:56.789012345Z
        // secs = 1704112496, subsec_nanos = 789_012_345
        // total nanos = 1_704_112_496_000_000_000 + 789_012_345
        //             = 1_704_112_496_789_012_345
        let total: i64 = 1_704_112_496 * 1_000_000_000 + 789_012_345;
        let dt = super::DatetimeIndex::new(vec![total, i64::MIN, 0]);

        assert_eq!(dt.hour(), vec![Some(12), None, Some(0)]);
        assert_eq!(dt.minute(), vec![Some(34), None, Some(0)]);
        assert_eq!(dt.second(), vec![Some(56), None, Some(0)]);
        assert_eq!(dt.microsecond(), vec![Some(789_012), None, Some(0)]);
        assert_eq!(dt.nanosecond(), vec![Some(345), None, Some(0)]);
    }

    #[test]
    fn datetime_index_time_of_day_indexers_match_pandas_bwzmn() -> Result<(), super::IndexError> {
        let hour = fp_types::Timedelta::NANOS_PER_HOUR;
        let minute = fp_types::Timedelta::NANOS_PER_MIN;
        let day = fp_types::Timedelta::NANOS_PER_DAY;
        let dt = super::DatetimeIndex::new(vec![
            9 * hour,
            12 * hour + 30 * minute,
            i64::MIN,
            23 * hour + 30 * minute,
            day + 30 * minute,
        ]);

        assert_eq!(dt.indexer_at_time("12:30")?, vec![1]);
        assert_eq!(dt.indexer_at_time("12:30:00.000000000")?, vec![1]);
        assert_eq!(dt.indexer_at_time("00:30:00")?, vec![4]);
        assert!(dt.indexer_at_time("not-a-time").is_err());

        assert_eq!(
            dt.indexer_between_time("08:00", "13:00", true, true)?,
            vec![0, 1]
        );
        assert_eq!(
            dt.indexer_between_time("09:00", "13:00", false, true)?,
            vec![1]
        );
        assert_eq!(
            dt.indexer_between_time("23:00", "01:00", true, true)?,
            vec![3, 4]
        );
        assert_eq!(
            dt.indexer_between_time("23:30", "00:30", false, false)?,
            Vec::<usize>::new()
        );
        assert!(
            dt.indexer_between_time("09:00", "not-a-time", true, true)
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn datetime_index_week_weekofyear_match_pandas_e8xhb() {
        const NS: i64 = 1_000_000_000;
        // 2024-01-01 (Monday) is in ISO week 1 of 2024.
        let jan_01 = 1_704_067_200_i64 * NS;
        // 2024-12-30 (Monday) is in ISO week 1 of 2025 (yes: pandas/ chrono
        // both report this as week 1).
        let dec_30 = 1_735_516_800_i64 * NS;
        let dt = super::DatetimeIndex::new(vec![jan_01, dec_30, i64::MIN]);

        let weeks = dt.week();
        assert_eq!(weeks[0], Some(1));
        assert_eq!(weeks[1], Some(1));
        assert_eq!(weeks[2], None);

        // weekofyear is an alias.
        assert_eq!(dt.weekofyear(), weeks);
        assert_eq!(
            dt.isocalendar(),
            vec![Some((2024, 1, 1)), Some((2025, 1, 1)), None]
        );
    }

    #[test]
    fn datetime_index_day_of_x_and_quarter_match_pandas_k860x() {
        // 2024-01-15T00:00:00Z (a Monday).
        let mon: i64 = 1_705_276_800 * 1_000_000_000;
        // 2024-01-21T00:00:00Z (a Sunday).
        let sun: i64 = 1_705_795_200 * 1_000_000_000;
        // 2024-04-30T00:00:00Z (Apr -> 30 days; Q2).
        let apr30: i64 = 1_714_435_200 * 1_000_000_000;
        let dt = super::DatetimeIndex::new(vec![mon, sun, apr30, i64::MIN]);

        // 2024 is a leap year. Jan 15 = ordinal 15. Jan 21 = ordinal 21.
        // Apr 30 = 31+29+31+30 = 121.
        assert_eq!(dt.dayofyear(), vec![Some(15), Some(21), Some(121), None]);
        assert_eq!(dt.day_of_year(), dt.dayofyear());

        // Mon=0, Sun=6. Apr 30 2024 was a Tuesday → 1.
        assert_eq!(dt.dayofweek(), vec![Some(0), Some(6), Some(1), None]);
        assert_eq!(dt.day_of_week(), dt.dayofweek());
        assert_eq!(dt.weekday(), dt.dayofweek());

        // Q1 / Q1 / Q2.
        assert_eq!(dt.quarter(), vec![Some(1), Some(1), Some(2), None]);

        // 2024 is a leap year.
        assert_eq!(
            dt.is_leap_year(),
            vec![Some(true), Some(true), Some(true), None]
        );

        // Jan -> 31, Apr -> 30.
        assert_eq!(dt.days_in_month(), vec![Some(31), Some(31), Some(30), None]);
        assert_eq!(dt.daysinmonth(), dt.days_in_month());
    }

    #[test]
    fn datetime_index_boundary_accessors_match_pandas_qy7yd() {
        // 2024 is a leap year. Each entry is the 00:00:00Z second-of-epoch
        // multiplied by 1_000_000_000.
        const NS: i64 = 1_000_000_000;
        let jan_01 = 1_704_067_200_i64 * NS; // year/quarter/month start
        let jan_31 = 1_706_659_200_i64 * NS; // month end (Jan)
        let feb_29 = 1_709_164_800_i64 * NS; // leap-month end
        let mar_31 = 1_711_843_200_i64 * NS; // quarter/month end (Q1)
        let apr_01 = 1_711_929_600_i64 * NS; // quarter/month start (Q2)
        let dec_31 = 1_735_603_200_i64 * NS; // year/quarter/month end
        let nat = i64::MIN;

        let dt =
            super::DatetimeIndex::new(vec![jan_01, jan_31, feb_29, mar_31, apr_01, dec_31, nat]);

        // is_year_start: only Jan 1.
        assert_eq!(
            dt.is_year_start(),
            vec![
                Some(true),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
                None
            ]
        );
        // is_year_end: only Dec 31.
        assert_eq!(
            dt.is_year_end(),
            vec![
                Some(false),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
                Some(true),
                None
            ]
        );
        // is_quarter_start: Jan 1, Apr 1.
        assert_eq!(
            dt.is_quarter_start(),
            vec![
                Some(true),
                Some(false),
                Some(false),
                Some(false),
                Some(true),
                Some(false),
                None
            ]
        );
        // is_quarter_end: Mar 31, Dec 31.
        assert_eq!(
            dt.is_quarter_end(),
            vec![
                Some(false),
                Some(false),
                Some(false),
                Some(true),
                Some(false),
                Some(true),
                None
            ]
        );
        // is_month_start: Jan 1, Apr 1.
        assert_eq!(
            dt.is_month_start(),
            vec![
                Some(true),
                Some(false),
                Some(false),
                Some(false),
                Some(true),
                Some(false),
                None
            ]
        );
        // is_month_end: Jan 31, Feb 29 (leap), Mar 31, Dec 31.
        assert_eq!(
            dt.is_month_end(),
            vec![
                Some(false),
                Some(true),
                Some(true),
                Some(true),
                Some(false),
                Some(true),
                None
            ]
        );
    }

    #[test]
    fn index_variants_insert_match_pandas_veabb() -> Result<(), super::IndexError> {
        const NS: i64 = 1_000_000_000;
        let a = 1_704_067_200_i64 * NS;
        let b = 1_705_276_800_i64 * NS;
        let c = 1_706_140_800_i64 * NS;
        let dt = super::DatetimeIndex::new(vec![a, c]).set_name("ts");

        // Middle insertion.
        let middle = dt.insert(1, b)?;
        assert_eq!(middle.values(), vec![Some(a), Some(b), Some(c)]);
        assert_eq!(middle.name(), Some("ts"));

        // End insertion (loc == len()).
        let end = dt.insert(dt.len(), b)?;
        assert_eq!(end.values(), vec![Some(a), Some(c), Some(b)]);

        // OOB.
        assert!(matches!(
            dt.insert(99, b).unwrap_err(),
            super::IndexError::OutOfBounds {
                position: 99,
                length: 2
            }
        ));

        let td = super::TimedeltaIndex::new(vec![100_i64, 300]).set_name("d");
        let td_inserted = td.insert(1, 200)?;
        assert_eq!(td_inserted.values(), vec![Some(100), Some(200), Some(300)]);
        assert_eq!(td_inserted.name(), Some("d"));

        use fp_types::{Period, PeriodFreq};
        let p1 = Period::new(10, PeriodFreq::Monthly);
        let p2 = Period::new(11, PeriodFreq::Monthly);
        let p3 = Period::new(12, PeriodFreq::Monthly);
        let pi = super::PeriodIndex::new(vec![p1, p3]).set_name("p");
        let pi_inserted = pi.insert(1, p2)?;
        assert_eq!(pi_inserted.values(), &[p1, p2, p3]);

        let r = super::RangeIndex::new(0, 3, 1).unwrap();
        let r_inserted = r.insert(1, 99)?;
        let labels = int64_labels(&r_inserted);
        assert_eq!(labels, vec![0, 99, 1, 2]);
        Ok(())
    }

    #[test]
    fn index_variants_format_match_pandas_n31q2() {
        const NS: i64 = 1_000_000_000;
        let dt = super::DatetimeIndex::new(vec![1_704_067_200_i64 * NS, i64::MIN]);
        let dt_fmt = dt.format();
        assert!(dt_fmt[0].starts_with("2024-01-01"));
        assert_eq!(dt_fmt[1], "NaT");

        let td = super::TimedeltaIndex::new(vec![1_000_000_i64, fp_types::Timedelta::NAT]);
        let td_fmt = td.format();
        assert_eq!(td_fmt[0], "1000000");
        assert_eq!(td_fmt[1], "NaT");

        use fp_types::{Period, PeriodFreq};
        let pi = super::PeriodIndex::new(vec![Period::new(10, PeriodFreq::Monthly)]);
        let pi_fmt = pi.format();
        assert!(!pi_fmt[0].is_empty());

        let cat = super::CategoricalIndex::from_values(vec!["a".to_owned(), "b".to_owned()], false);
        assert_eq!(cat.format(), vec!["a".to_owned(), "b".to_owned()]);
    }

    #[test]
    fn datetime_timedelta_fillna_isnull_match_pandas_az3t9() {
        const NS: i64 = 1_000_000_000;
        let unix = 1_704_067_200_i64 * NS;
        let dt = super::DatetimeIndex::new(vec![unix, i64::MIN, 0]).set_name("ts");

        let filled = dt.fillna(unix);
        // NAT is replaced; existing values are preserved.
        assert_eq!(filled.values(), vec![Some(unix), Some(unix), Some(0)]);
        assert_eq!(filled.name(), Some("ts"));

        let iso = dt.isnull();
        assert_eq!(iso, dt.isna());
        let nio = dt.notnull();
        assert_eq!(nio, dt.notna());

        let nat = fp_types::Timedelta::NAT;
        let td = super::TimedeltaIndex::new(vec![100_i64, nat, 0]).set_name("d");
        let td_filled = td.fillna(99);
        assert_eq!(td_filled.values(), vec![Some(100), Some(99), Some(0)]);
        assert_eq!(td_filled.name(), Some("d"));
        assert_eq!(td.isnull(), td.isna());
        assert_eq!(td.notnull(), td.notna());
    }

    #[test]
    fn datetime_index_date_and_time_accessors_match_pandas_66pll() {
        const NS: i64 = 1_000_000_000;
        // 2024-01-15T12:34:56.789012345Z (computed in br-teeck).
        let total: i64 = 1_705_322_096_i64 * NS + 789_012_345;
        let dt = super::DatetimeIndex::new(vec![total, i64::MIN, 0]);

        let dates = dt.date();
        assert_eq!(
            dates[0],
            Some(chrono::NaiveDate::from_ymd_opt(2024, 1, 15).unwrap())
        );
        assert_eq!(dates[1], None);
        assert_eq!(
            dates[2],
            Some(chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())
        );

        let times = dt.time();
        assert_eq!(
            times[0],
            chrono::NaiveTime::from_hms_nano_opt(12, 34, 56, 789_012_345)
        );
        assert_eq!(times[1], None);
        assert_eq!(times[2], chrono::NaiveTime::from_hms_nano_opt(0, 0, 0, 0));
        assert_eq!(dt.timetz(), times);
    }

    #[test]
    fn datetime_index_to_pydatetime_and_julian_match_pandas_dww6m() {
        const NS: i64 = 1_000_000_000;
        // 2024-01-01T00:00:00Z
        let unix = 1_704_067_200_i64;
        let total = unix * NS;
        let dt = super::DatetimeIndex::new(vec![total, i64::MIN]);

        let pydt = dt.to_pydatetime();
        let first = pydt[0].expect("non-NAT label decodes");
        assert_eq!(first.timestamp(), unix);
        assert_eq!(pydt[1], None);

        let julian = dt.to_julian_date();
        // JD for 2024-01-01T00:00:00Z = 2_460_310.5.
        let expected = (unix as f64) / 86_400.0 + 2_440_587.5;
        let observed = julian[0].expect("non-NAT label decodes");
        assert!((observed - expected).abs() < 1e-9);
        assert_eq!(julian[1], None);
    }

    #[test]
    fn timedelta_index_to_pytimedelta_match_pandas_dww6m() {
        let one_day_nanos = fp_types::Timedelta::NANOS_PER_DAY;
        let td = super::TimedeltaIndex::new(vec![one_day_nanos, fp_types::Timedelta::NAT]);
        let durations = td.to_pytimedelta();
        let one_day = durations[0].expect("non-NAT label decodes");
        assert_eq!(one_day.num_seconds(), 86_400);
        assert_eq!(durations[1], None);
    }

    #[test]
    fn datetime_index_tz_localize_tz_convert_match_pandas_qm31w() {
        const NS: i64 = 1_000_000_000;
        let dt = super::DatetimeIndex::new(vec![1_704_067_200_i64 * NS]).set_name("ts");

        // UTC is a no-op clone.
        let utc = dt.tz_localize("UTC").expect("UTC localize");
        assert!(utc.equals(&dt));
        assert_eq!(utc.name(), Some("ts"));

        // Other timezones reject.
        let err = dt.tz_localize("US/Eastern").unwrap_err();
        assert!(matches!(
            err,
            super::IndexError::InvalidArgument(ref message)
                if message.contains("tz_localize") && message.contains("UTC")
        ));

        // tz_convert always rejects.
        let conv_err = dt.tz_convert("UTC").unwrap_err();
        assert!(matches!(
            conv_err,
            super::IndexError::InvalidArgument(ref message)
                if message.contains("tz_convert")
        ));
    }

    #[test]
    fn datetime_timedelta_as_unit_match_pandas_70mbe() {
        let dt = super::DatetimeIndex::new(vec![]);
        assert!(dt.as_unit("ns").is_ok());
        let bad = dt.as_unit("us").unwrap_err();
        assert!(matches!(
            bad,
            super::IndexError::InvalidArgument(ref msg) if msg.contains("as_unit")
        ));

        let td = super::TimedeltaIndex::new(vec![]);
        assert!(td.as_unit("ns").is_ok());
        assert!(td.as_unit("ms").is_err());
    }

    #[test]
    fn datetime_timedelta_unit_resolution_match_pandas_c50rv() {
        let dt = super::DatetimeIndex::new(vec![]);
        assert_eq!(dt.unit(), "ns");
        assert_eq!(dt.resolution(), "nanosecond");

        let td = super::TimedeltaIndex::new(vec![]);
        assert_eq!(td.unit(), "ns");
        assert_eq!(td.resolution(), "nanosecond");
    }

    #[test]
    fn datetime_timedelta_tz_freq_accessors_return_none_ze7et() {
        const NS: i64 = 1_000_000_000;
        let dt = super::DatetimeIndex::new(vec![1_704_067_200_i64 * NS]);
        assert_eq!(dt.tz(), None);
        assert_eq!(dt.tzinfo(), None);
        assert_eq!(dt.freq(), None);
        assert_eq!(dt.freqstr(), None);
        assert_eq!(dt.inferred_freq(), None);

        let td = super::TimedeltaIndex::new(vec![100_i64]);
        assert_eq!(td.freq(), None);
        assert_eq!(td.freqstr(), None);
        assert_eq!(td.inferred_freq(), None);
    }

    #[test]
    fn period_index_freqstr_inferred_freq_match_pandas_ze7et() {
        use fp_types::{Period, PeriodFreq};
        let pi = super::PeriodIndex::new(vec![
            Period::new(10, PeriodFreq::Monthly),
            Period::new(11, PeriodFreq::Monthly),
        ]);
        let s = pi.freqstr().expect("homogeneous index has a freqstr");
        assert!(!s.is_empty());
        let inferred = pi.inferred_freq().expect("homogeneous freq is inferable");
        assert_eq!(inferred, s);

        // Mixed-frequency index: inferred_freq returns None.
        let mixed = super::PeriodIndex::new(vec![
            Period::new(10, PeriodFreq::Monthly),
            Period::new(10, PeriodFreq::Annual),
        ]);
        assert_eq!(mixed.inferred_freq(), None);

        // Empty index: freqstr is None.
        let empty = super::PeriodIndex::new(Vec::new());
        assert_eq!(empty.freqstr(), None);
        assert_eq!(empty.inferred_freq(), None);
    }

    #[test]
    fn range_index_where_putmask_match_pandas_jw1kw() -> Result<(), super::IndexError> {
        let r = super::RangeIndex::new(0, 5, 1).unwrap().set_name("r");

        let masked = r.r#where(&[true, false, true, false, true], 99)?;
        assert_eq!(int64_labels(&masked), vec![0, 99, 2, 99, 4]);
        assert_eq!(masked.name(), Some("r"));

        let put = r.putmask(&[false, true, false, true, false], 99)?;
        assert_eq!(int64_labels(&put), vec![0, 99, 2, 99, 4]);

        // Length mismatch.
        assert!(matches!(
            r.r#where(&[true, false], 0).unwrap_err(),
            super::IndexError::LengthMismatch { .. }
        ));
        assert!(matches!(
            r.putmask(&[true; 7], 0).unwrap_err(),
            super::IndexError::LengthMismatch { .. }
        ));
        Ok(())
    }

    #[test]
    fn range_index_set_ops_match_pandas_tz40f() {
        let left = super::RangeIndex::new(0, 5, 1).unwrap().set_name("r");
        let right = super::RangeIndex::new(3, 8, 1).unwrap().set_name("r");

        let inter = left.intersection(&right);
        assert_eq!(int64_labels(&inter), vec![3, 4]);
        assert_eq!(inter.name(), Some("r"));

        let union = left.union(&right);
        assert_eq!(int64_labels(&union), vec![0, 1, 2, 3, 4, 5, 6, 7]);

        let diff = left.difference(&right);
        assert_eq!(int64_labels(&diff), vec![0, 1, 2]);

        let sym = left.symmetric_difference(&right);
        assert_eq!(int64_labels(&sym), vec![0, 1, 2, 5, 6, 7]);

        // Mismatched names drop the name.
        let other_name = super::RangeIndex::new(3, 6, 1).unwrap().set_name("other");
        assert_eq!(left.union(&other_name).name(), None);
    }

    #[test]
    fn period_range_slice_indexer_match_pandas_18kvv() -> Result<(), super::IndexError> {
        use fp_types::{Period, PeriodFreq};
        let pi = super::PeriodIndex::new(vec![
            Period::new(10, PeriodFreq::Monthly),
            Period::new(11, PeriodFreq::Monthly),
            Period::new(12, PeriodFreq::Monthly),
        ]);
        assert_eq!(
            pi.slice_indexer(
                Period::new(11, PeriodFreq::Monthly),
                Period::new(12, PeriodFreq::Monthly)
            )?,
            1..3
        );

        let r = super::RangeIndex::new(0, 10, 2).unwrap();
        assert_eq!(r.slice_indexer(2, 6)?, 1..4);
        Ok(())
    }

    #[test]
    fn period_range_slice_locs_match_pandas_fdga0() -> Result<(), super::IndexError> {
        use fp_types::{Period, PeriodFreq};
        let p1 = Period::new(10, PeriodFreq::Monthly);
        let p2 = Period::new(11, PeriodFreq::Monthly);
        let p3 = Period::new(12, PeriodFreq::Monthly);
        let p4 = Period::new(13, PeriodFreq::Monthly);
        let pi = super::PeriodIndex::new(vec![p1, p2, p3, p4]);
        assert_eq!(pi.slice_locs(p2, p3)?, (1, 3));
        assert_eq!(pi.slice_locs(p1, p4)?, (0, 4));
        // Non-monotonic rejects.
        let unsorted = super::PeriodIndex::new(vec![p3, p1, p2]);
        assert!(unsorted.slice_locs(p1, p3).is_err());

        let r = super::RangeIndex::new(0, 10, 2).unwrap();
        // Values 0,2,4,6,8.
        assert_eq!(r.slice_locs(2, 6)?, (1, 4));
        assert_eq!(r.slice_locs(0, 8)?, (0, 5));

        // Descending rejects.
        let desc = super::RangeIndex::new(10, 0, -2).unwrap();
        assert!(desc.slice_locs(2, 6).is_err());
        Ok(())
    }

    #[test]
    fn typed_index_variants_rename_alias_match_pandas_i8t6n() {
        let dt = super::DatetimeIndex::new(vec![]);
        assert_eq!(dt.rename("ts").name(), Some("ts"));

        let td = super::TimedeltaIndex::new(vec![]);
        assert_eq!(td.rename("d").name(), Some("d"));

        use fp_types::PeriodFreq;
        let pi = super::PeriodIndex::new(vec![]);
        assert_eq!(pi.rename("p").name(), Some("p"));
        let _ = PeriodFreq::Monthly; // suppress unused-import warning when no other test in scope

        let r = super::RangeIndex::new(0, 0, 1).unwrap();
        assert_eq!(r.rename("r").name(), Some("r"));

        let cat = super::CategoricalIndex::from_values(vec!["a".to_owned()], false);
        assert_eq!(cat.rename("c").name(), Some("c"));
    }

    #[test]
    fn typed_index_variants_reindex_match_pandas_qm3nq() {
        const NS: i64 = 1_000_000_000;
        let a = 1_704_067_200_i64 * NS;
        let b = 1_705_276_800_i64 * NS;
        let dt = super::DatetimeIndex::new(vec![a, b]);
        let target = super::DatetimeIndex::new(vec![b, a, 0]);
        let (out, indexer) = dt.reindex(&target);
        assert_eq!(out.values(), target.values());
        assert_eq!(indexer, vec![1, 0, -1]);

        let td = super::TimedeltaIndex::new(vec![100_i64, 200]);
        let td_target = super::TimedeltaIndex::new(vec![200_i64, 999]);
        let (_, td_indexer) = td.reindex(&td_target);
        assert_eq!(td_indexer, vec![1, -1]);

        use fp_types::{Period, PeriodFreq};
        let p1 = Period::new(10, PeriodFreq::Monthly);
        let p2 = Period::new(11, PeriodFreq::Monthly);
        let pi = super::PeriodIndex::new(vec![p1, p2]);
        let pi_target = super::PeriodIndex::new(vec![p2, Period::new(99, PeriodFreq::Monthly)]);
        let (_, pi_indexer) = pi.reindex(&pi_target);
        assert_eq!(pi_indexer, vec![1, -1]);

        let r = super::RangeIndex::new(0, 5, 1).unwrap();
        let r_target = super::RangeIndex::new(2, 6, 1).unwrap();
        let (_, r_indexer) = r.reindex(&r_target);
        assert_eq!(r_indexer, vec![2, 3, 4, -1]);
    }

    #[test]
    fn period_range_categorical_get_indexer_non_unique_match_pandas_z9sna()
    -> Result<(), super::IndexError> {
        use fp_types::{Period, PeriodFreq};
        let p1 = Period::new(10, PeriodFreq::Monthly);
        let p2 = Period::new(11, PeriodFreq::Monthly);
        // PeriodIndex with duplicate p1.
        let pi = super::PeriodIndex::new(vec![p1, p2, p1]);
        let (positions, missing) =
            pi.get_indexer_non_unique(&[p1, Period::new(99, PeriodFreq::Monthly)]);
        assert_eq!(positions, vec![0, 2, -1]);
        assert_eq!(missing, vec![1]);

        // RangeIndex (always unique).
        let r = super::RangeIndex::new(0, 5, 1).unwrap();
        let (positions, missing) = r.get_indexer_non_unique(&[2, 99]);
        assert_eq!(positions, vec![2, -1]);
        assert_eq!(missing, vec![1]);

        // CategoricalIndex with duplicate "a".
        let cat = super::CategoricalIndex::from_values(
            vec!["a".to_owned(), "b".to_owned(), "a".to_owned()],
            false,
        );
        let (positions, missing) = cat.get_indexer_non_unique(&["a".to_owned(), "z".to_owned()]);
        assert_eq!(positions, vec![0, 2, -1]);
        assert_eq!(missing, vec![1]);

        // Categorical get_indexer also works.
        let mapped = cat.get_indexer(&["b".to_owned(), "z".to_owned()]);
        assert_eq!(mapped, vec![1, -1]);
        // get_indexer_for is an alias.
        assert_eq!(
            cat.get_indexer_for(&["a".to_owned()]),
            cat.get_indexer(&["a".to_owned()])
        );
        Ok(())
    }

    #[test]
    fn typed_index_variants_get_indexer_for_aliases_match_pandas_lf1jy()
    -> Result<(), super::IndexError> {
        const NS: i64 = 1_000_000_000;
        let dt = super::DatetimeIndex::new(vec![1_704_067_200_i64 * NS]);
        assert_eq!(
            dt.get_indexer_for(&[1_704_067_200_i64 * NS, 0]),
            dt.get_indexer(&[1_704_067_200_i64 * NS, 0])
        );

        let td = super::TimedeltaIndex::new(vec![100_i64, 200]);
        assert_eq!(td.get_indexer_for(&[200, 999]), td.get_indexer(&[200, 999]));

        use fp_types::{Period, PeriodFreq};
        let pi = super::PeriodIndex::new(vec![Period::new(10, PeriodFreq::Monthly)]);
        let target = vec![Period::new(10, PeriodFreq::Monthly)];
        assert_eq!(pi.get_indexer_for(&target), pi.get_indexer(&target));

        let r = super::RangeIndex::new(0, 5, 1).unwrap();
        assert_eq!(r.get_indexer_for(&[2, 99]), r.get_indexer(&[2, 99]));
        Ok(())
    }

    #[test]
    fn period_range_get_loc_get_indexer_match_pandas_e7psu() -> Result<(), super::IndexError> {
        use fp_types::{Period, PeriodFreq};
        let p1 = Period::new(10, PeriodFreq::Monthly);
        let p2 = Period::new(11, PeriodFreq::Monthly);
        let p3 = Period::new(12, PeriodFreq::Monthly);
        let pi = super::PeriodIndex::new(vec![p1, p2, p3]);
        assert_eq!(pi.get_loc(p2)?, 1);
        assert!(pi.get_loc(Period::new(99, PeriodFreq::Monthly)).is_err());
        assert_eq!(
            pi.get_indexer(&[p3, p1, Period::new(99, PeriodFreq::Monthly)]),
            vec![2, 0, -1]
        );

        // RangeIndex with step=2: 0, 2, 4, 6, 8.
        let r = super::RangeIndex::new(0, 10, 2).unwrap();
        assert_eq!(r.get_loc(0)?, 0);
        assert_eq!(r.get_loc(8)?, 4);
        assert!(r.get_loc(7).is_err()); // not in step
        assert!(r.get_loc(99).is_err()); // out of range
        assert_eq!(r.get_indexer(&[4, 7, 0, 99]), vec![2, -1, 0, -1]);

        // Negative step: 10, 8, 6, 4, 2.
        let desc = super::RangeIndex::new(10, 0, -2).unwrap();
        assert_eq!(desc.get_loc(10)?, 0);
        assert_eq!(desc.get_loc(2)?, 4);
        assert!(desc.get_loc(7).is_err());
        Ok(())
    }

    #[test]
    fn period_index_where_putmask_match_pandas_so9oh() -> Result<(), super::IndexError> {
        use fp_types::{Period, PeriodFreq};
        let p1 = Period::new(10, PeriodFreq::Monthly);
        let p2 = Period::new(11, PeriodFreq::Monthly);
        let p3 = Period::new(12, PeriodFreq::Monthly);
        let pi = super::PeriodIndex::new(vec![p1, p2, p3]).set_name("p");

        // where: keep position 0 and 2.
        let masked = pi.r#where(&[true, false, true], p1)?;
        assert_eq!(masked.values(), &[p1, p1, p3]);
        assert_eq!(masked.name(), Some("p"));

        // putmask: replace masked positions.
        let put = pi.putmask(&[false, true, false], p1)?;
        assert_eq!(put.values(), &[p1, p1, p3]);

        // Length mismatch.
        let bad_len = pi.r#where(&[true, false], p1).unwrap_err();
        assert!(matches!(bad_len, super::IndexError::LengthMismatch { .. }));

        // Mismatched freq replacement rejects.
        let mismatch = Period::new(10, PeriodFreq::Annual);
        assert!(pi.r#where(&[true, false, true], mismatch).is_err());
        assert!(pi.putmask(&[false, true, false], mismatch).is_err());
        Ok(())
    }

    #[test]
    fn categorical_index_where_putmask_match_pandas_so9oh() -> Result<(), super::IndexError> {
        let cat = super::CategoricalIndex::with_categories(
            vec!["a".to_owned(), "b".to_owned(), "c".to_owned()],
            vec![
                "a".to_owned(),
                "b".to_owned(),
                "c".to_owned(),
                "d".to_owned(),
            ],
            false,
        )?;

        let masked = cat.r#where(&[true, false, true], "d")?;
        assert_eq!(
            masked.labels(),
            vec!["a".to_owned(), "d".to_owned(), "c".to_owned()].as_slice()
        );

        let put = cat.putmask(&[false, true, true], "d")?;
        assert_eq!(
            put.labels(),
            vec!["a".to_owned(), "d".to_owned(), "d".to_owned()].as_slice()
        );

        // Replacement that's not a category rejects.
        assert!(cat.r#where(&[true, false, true], "zzz").is_err());

        // Length mismatch.
        assert!(cat.putmask(&[true; 5], "a").is_err());
        Ok(())
    }

    #[test]
    fn period_index_set_ops_match_pandas_8042v() -> Result<(), super::IndexError> {
        use fp_types::{Period, PeriodFreq};
        let p1 = Period::new(10, PeriodFreq::Monthly);
        let p2 = Period::new(11, PeriodFreq::Monthly);
        let p3 = Period::new(12, PeriodFreq::Monthly);
        let p4 = Period::new(13, PeriodFreq::Monthly);
        let left = super::PeriodIndex::new(vec![p1, p2, p3]).set_name("p");
        let right = super::PeriodIndex::new(vec![p2, p3, p4]).set_name("p");

        assert_eq!(left.intersection(&right)?.values(), &[p2, p3]);
        assert_eq!(left.union(&right)?.values(), &[p1, p2, p3, p4]);
        assert_eq!(left.difference(&right)?.values(), &[p1]);
        assert_eq!(left.symmetric_difference(&right)?.values(), &[p1, p4]);

        // Mismatched freq rejects.
        let mismatch = super::PeriodIndex::new(vec![Period::new(10, PeriodFreq::Annual)]);
        assert!(left.intersection(&mismatch).is_err());
        assert!(left.union(&mismatch).is_err());
        assert!(left.difference(&mismatch).is_err());
        assert!(left.symmetric_difference(&mismatch).is_err());

        // Mismatched names drop the name.
        let other_name = super::PeriodIndex::new(vec![p2]).set_name("other");
        assert_eq!(left.union(&other_name)?.name(), None);
        Ok(())
    }

    #[test]
    fn period_categorical_sort_values_match_pandas_482qd() -> Result<(), super::IndexError> {
        use fp_types::{Period, PeriodFreq};
        let p1 = Period::new(10, PeriodFreq::Monthly);
        let p2 = Period::new(11, PeriodFreq::Monthly);
        let p3 = Period::new(12, PeriodFreq::Monthly);
        let pi = super::PeriodIndex::new(vec![p3, p1, p2]).set_name("p");
        let sorted = pi.sort_values()?;
        let sorted_alias = pi.sort()?;
        assert_eq!(sorted.values(), &[p1, p2, p3]);
        assert_eq!(sorted_alias.values(), sorted.values());
        assert_eq!(sorted.name(), Some("p"));
        assert_eq!(sorted_alias.name(), Some("p"));

        let mixed = super::PeriodIndex::new(vec![
            Period::new(10, PeriodFreq::Monthly),
            Period::new(10, PeriodFreq::Annual),
        ]);
        assert!(mixed.sort_values().is_err());
        assert!(mixed.sort().is_err());

        // CategoricalIndex with ordered=true uses category position.
        let cat = super::CategoricalIndex::with_categories(
            vec![
                "b".to_owned(),
                "a".to_owned(),
                "c".to_owned(),
                "a".to_owned(),
            ],
            vec!["a".to_owned(), "b".to_owned(), "c".to_owned()],
            true,
        )?;
        let cat_sorted = cat.sort_values();
        let cat_sorted_alias = cat.sort();
        assert_eq!(
            cat_sorted.labels(),
            vec![
                "a".to_owned(),
                "a".to_owned(),
                "b".to_owned(),
                "c".to_owned()
            ]
            .as_slice()
        );
        assert_eq!(cat_sorted_alias.labels(), cat_sorted.labels());

        Ok(())
    }

    #[test]
    fn categorical_sort_values_by_category_code_not_lexicographic() {
        // Non-lexicographic category order: codes b=0, a=1, c=2. pandas
        // sort_values orders by code -> b, a, a, c (NOT lexicographic a,a,b,c).
        // The old `to_index().argsort()` text sort returned a,a,b,c, which
        // diverges from pandas; the existing 482qd test used lexicographic
        // categories so it could not catch this.
        let cat = super::CategoricalIndex::with_categories(
            vec![
                "a".to_owned(),
                "c".to_owned(),
                "b".to_owned(),
                "a".to_owned(),
            ],
            vec!["b".to_owned(), "a".to_owned(), "c".to_owned()],
            true,
        )
        .unwrap();
        // labels [a,c,b,a] -> codes [1,2,0,1]; stable argsort by code:
        // code0 -> pos2 (b); code1 -> pos0 (a), pos3 (a); code2 -> pos1 (c).
        assert_eq!(cat.argsort(), vec![2, 0, 3, 1]);
        assert_eq!(
            cat.sort_values().labels(),
            [
                "b".to_owned(),
                "a".to_owned(),
                "a".to_owned(),
                "c".to_owned()
            ]
            .as_slice()
        );

        // Unordered categoricals also sort by category code in pandas.
        let cat_u = super::CategoricalIndex::with_categories(
            vec!["a".to_owned(), "b".to_owned()],
            vec!["b".to_owned(), "a".to_owned()],
            false,
        )
        .unwrap();
        // codes a=1, b=0 -> sorted by code: b, a.
        assert_eq!(
            cat_u.sort_values().labels(),
            ["b".to_owned(), "a".to_owned()].as_slice()
        );
    }

    #[test]
    fn period_index_from_ordinals_match_pandas_baenb() {
        use fp_types::PeriodFreq;
        let pi = super::PeriodIndex::from_ordinals(&[10, 11, 12], PeriodFreq::Monthly);
        assert_eq!(pi.values().len(), 3);
        assert_eq!(pi.values()[0].ordinal, 10);
        assert_eq!(pi.values()[2].ordinal, 12);
        assert_eq!(pi.asi8(), vec![10, 11, 12]);
        for period in pi.values() {
            assert_eq!(period.freq, PeriodFreq::Monthly);
        }

        let empty = super::PeriodIndex::from_ordinals(&[], PeriodFreq::Annual);
        assert!(empty.is_empty());
        assert!(empty.asi8().is_empty());
    }

    #[test]
    fn period_index_astype_datetime_and_int_match_pandas() -> Result<(), super::IndexError> {
        use fp_types::PeriodFreq;

        let pi = super::PeriodIndex::from_ordinals(&[600, 601], PeriodFreq::Monthly).set_name("p");

        let as_int = pi.astype("int64")?;
        assert_eq!(
            as_int.labels(),
            &[IndexLabel::Int64(600), IndexLabel::Int64(601)]
        );
        assert_eq!(as_int.name(), Some("p"));

        let as_datetime = pi.astype("datetime64[ns]")?;
        assert_eq!(
            as_datetime.labels(),
            &[
                IndexLabel::Datetime64(1_577_836_800_000_000_000),
                IndexLabel::Datetime64(1_580_515_200_000_000_000),
            ]
        );
        assert_eq!(as_datetime.name(), Some("p"));

        Ok(())
    }

    #[test]
    fn period_index_missing_value_accessors_are_all_present() {
        use fp_types::PeriodFreq;
        let pi = super::PeriodIndex::from_ordinals(&[10, 11, 12], PeriodFreq::Monthly)
            .set_name("periods");
        assert!(!pi.hasnans());
        assert_eq!(pi.isna(), vec![false, false, false]);
        assert_eq!(pi.isnull(), pi.isna());
        assert_eq!(pi.notna(), vec![true, true, true]);
        assert_eq!(pi.notnull(), pi.notna());
        let dropped = pi.dropna();
        assert_eq!(dropped.values(), pi.values());
        assert_eq!(dropped.name(), Some("periods"));
    }

    #[test]
    fn period_index_mean_median_match_pandas_3rsrc() -> Result<(), super::IndexError> {
        use fp_types::{Period, PeriodFreq};
        let p1 = Period::new(10, PeriodFreq::Monthly);
        let p2 = Period::new(20, PeriodFreq::Monthly);
        let p3 = Period::new(30, PeriodFreq::Monthly);
        let pi = super::PeriodIndex::new(vec![p1, p2, p3]);
        assert_eq!(pi.mean()?.unwrap().ordinal, 20);
        assert_eq!(pi.median()?.unwrap().ordinal, 20);

        let empty = super::PeriodIndex::new(Vec::new());
        assert_eq!(empty.mean()?, None);
        assert_eq!(empty.median()?, None);

        let mixed = super::PeriodIndex::new(vec![p1, Period::new(10, PeriodFreq::Annual)]);
        assert!(mixed.mean().is_err());
        assert!(mixed.median().is_err());
        Ok(())
    }

    #[test]
    fn period_index_argmax_argmin_argsort_match_pandas_qg8u5() -> Result<(), super::IndexError> {
        use fp_types::{Period, PeriodFreq};
        let p1 = Period::new(10, PeriodFreq::Monthly);
        let p2 = Period::new(11, PeriodFreq::Monthly);
        let p3 = Period::new(12, PeriodFreq::Monthly);
        let pi = super::PeriodIndex::new(vec![p2, p3, p1]);

        assert_eq!(pi.argmax()?, 1);
        assert_eq!(pi.argmin()?, 2);
        assert_eq!(pi.argsort()?, vec![2, 0, 1]);

        let empty = super::PeriodIndex::new(Vec::new());
        assert!(empty.argmax().is_err());
        assert!(empty.argmin().is_err());
        assert!(empty.argsort()?.is_empty());

        let mixed = super::PeriodIndex::new(vec![p1, Period::new(10, PeriodFreq::Annual)]);
        assert!(mixed.argmax().is_err());
        assert!(mixed.argsort().is_err());
        Ok(())
    }

    #[test]
    fn period_index_shift_match_pandas_pnaui() -> Result<(), super::IndexError> {
        use fp_types::{Period, PeriodFreq};
        let p1 = Period::new(10, PeriodFreq::Monthly);
        let p2 = Period::new(11, PeriodFreq::Monthly);
        let pi = super::PeriodIndex::new(vec![p1, p2]).set_name("p");

        let shifted = pi.shift(2)?;
        assert_eq!(shifted.values()[0].ordinal, 12);
        assert_eq!(shifted.values()[1].ordinal, 13);
        assert_eq!(shifted.name(), Some("p"));

        // Negative shift.
        let back = pi.shift(-1)?;
        assert_eq!(back.values()[0].ordinal, 9);

        // Mixed-freq rejects.
        let mixed = super::PeriodIndex::new(vec![p1, Period::new(10, PeriodFreq::Annual)]);
        assert!(mixed.shift(1).is_err());
        Ok(())
    }

    #[test]
    fn period_index_is_full_match_pandas_7i32m() {
        use fp_types::{Period, PeriodFreq};
        let p1 = Period::new(10, PeriodFreq::Monthly);
        let p2 = Period::new(11, PeriodFreq::Monthly);
        let p3 = Period::new(12, PeriodFreq::Monthly);
        let p5 = Period::new(14, PeriodFreq::Monthly);

        // Contiguous.
        let full = super::PeriodIndex::new(vec![p1, p2, p3]);
        assert!(full.is_full());

        // Out-of-order but contiguous (sort first).
        let unsorted = super::PeriodIndex::new(vec![p3, p1, p2]);
        assert!(unsorted.is_full());

        // Gap.
        let gap = super::PeriodIndex::new(vec![p1, p2, p5]);
        assert!(!gap.is_full());

        // Empty / single-element.
        assert!(super::PeriodIndex::new(Vec::new()).is_full());
        assert!(super::PeriodIndex::new(vec![p1]).is_full());

        // Mixed-frequency.
        let mixed = super::PeriodIndex::new(vec![p1, Period::new(10, PeriodFreq::Annual)]);
        assert!(!mixed.is_full());
    }

    #[test]
    fn period_index_min_max_match_pandas_fwlv4() -> Result<(), super::IndexError> {
        use fp_types::{Period, PeriodFreq};
        let p1 = Period::new(10, PeriodFreq::Monthly);
        let p2 = Period::new(11, PeriodFreq::Monthly);
        let p3 = Period::new(12, PeriodFreq::Monthly);
        let pi = super::PeriodIndex::new(vec![p3, p1, p2]);
        assert_eq!(pi.min()?, Some(p1));
        assert_eq!(pi.max()?, Some(p3));

        let empty = super::PeriodIndex::new(Vec::new());
        assert_eq!(empty.min()?, None);
        assert_eq!(empty.max()?, None);

        // Mixed freq rejects.
        let mixed = super::PeriodIndex::new(vec![
            Period::new(10, PeriodFreq::Monthly),
            Period::new(10, PeriodFreq::Annual),
        ]);
        assert!(mixed.min().is_err());
        assert!(mixed.max().is_err());
        Ok(())
    }

    #[test]
    fn range_index_sort_values_closed_form_mhcge() {
        let asc = super::RangeIndex::new(0, 5, 1).unwrap();
        assert!(asc.sort_values().equals(&asc));
        assert!(asc.sort().equals(&asc));

        let desc = super::RangeIndex::new(10, 0, -2).unwrap();
        // Original values 10, 8, 6, 4, 2 → sorted ascending 2, 4, 6, 8, 10.
        let sorted = desc.sort_values();
        let sorted_alias = desc.sort();
        assert_eq!(sorted.values(), vec![2, 4, 6, 8, 10]);
        assert_eq!(sorted_alias.values(), sorted.values());

        let empty = super::RangeIndex::new(0, 0, 1).unwrap();
        assert!(empty.sort_values().is_empty());
        assert!(empty.sort().is_empty());

        let zero_step = super::RangeIndex::new(0, 5, 1).unwrap();
        assert!(zero_step.sort_values().equals(&zero_step));
        assert!(zero_step.sort().equals(&zero_step));
    }

    #[test]
    fn range_index_std_var_median_closed_form_tkc0m() {
        let r = super::RangeIndex::new(1, 11, 1).unwrap();
        // 1..=10: median = 5.5; var = sum((x - 5.5)^2)/9; std = sqrt(var).
        assert_eq!(r.median(), Some(5.5));
        let var = r.var().unwrap();
        // Expected variance for 1..=10 is 9.166666...
        assert!((var - 9.1666666666).abs() < 1e-6);
        let std_val = r.std().unwrap();
        assert!((std_val - var.sqrt()).abs() < 1e-12);

        // Single element: var/std None; median = the value.
        let one = super::RangeIndex::new(5, 6, 1).unwrap();
        assert_eq!(one.median(), Some(5.0));
        assert_eq!(one.var(), None);
        assert_eq!(one.std(), None);

        // Empty: all None.
        let empty = super::RangeIndex::new(0, 0, 1).unwrap();
        assert_eq!(empty.median(), None);
        assert_eq!(empty.var(), None);
    }

    #[test]
    fn range_index_prod_match_pandas_8yxw8() {
        // 1..=5 prod = 120.
        let r = super::RangeIndex::new(1, 6, 1).unwrap();
        assert_eq!(r.prod(), 120);

        // Empty prod = 1.
        let empty = super::RangeIndex::new(0, 0, 1).unwrap();
        assert_eq!(empty.prod(), 1);

        // Includes zero → prod = 0.
        let with_zero = super::RangeIndex::new(0, 5, 1).unwrap();
        assert_eq!(with_zero.prod(), 0);
    }

    #[test]
    fn range_index_min_max_sum_mean_closed_form_fwlv4() {
        let asc = super::RangeIndex::new(1, 11, 1).unwrap();
        // Values 1..=10
        assert_eq!(asc.min(), Some(1));
        assert_eq!(asc.max(), Some(10));
        assert_eq!(asc.sum(), 55);
        assert_eq!(asc.mean(), Some(5.5));

        let desc = super::RangeIndex::new(10, 0, -2).unwrap();
        // Values 10, 8, 6, 4, 2 — sum=30, mean=6, min=2, max=10
        assert_eq!(desc.min(), Some(2));
        assert_eq!(desc.max(), Some(10));
        assert_eq!(desc.sum(), 30);
        assert_eq!(desc.mean(), Some(6.0));

        let empty = super::RangeIndex::new(0, 0, 1).unwrap();
        assert_eq!(empty.min(), None);
        assert_eq!(empty.max(), None);
        assert_eq!(empty.sum(), 0);
        assert_eq!(empty.mean(), None);
    }

    #[test]
    fn datetime_index_where_putmask_match_pandas_nwqty() -> Result<(), super::IndexError> {
        const NS: i64 = 1_000_000_000;
        let a = 1_704_067_200_i64 * NS;
        let b = 1_705_276_800_i64 * NS;
        let c = 1_706_140_800_i64 * NS;
        let dt = super::DatetimeIndex::new(vec![a, b, c]).set_name("ts");

        // where: keep position 0 and 2; replace position 1 with i64::MIN (NAT).
        let masked = dt.r#where(&[true, false, true], i64::MIN)?;
        assert_eq!(masked.values(), vec![Some(a), None, Some(c)]);
        assert_eq!(masked.name(), Some("ts"));

        // putmask: replace positions where mask=true with c.
        let put = dt.putmask(&[true, false, false], c)?;
        assert_eq!(put.values(), vec![Some(c), Some(b), Some(c)]);

        // Length mismatch errors.
        let bad_cond = dt.r#where(&[true, false], i64::MIN).unwrap_err();
        assert!(matches!(
            bad_cond,
            super::IndexError::LengthMismatch {
                expected: 3,
                actual: 2,
                ..
            }
        ));
        let bad_mask = dt.putmask(&[true; 5], c).unwrap_err();
        assert!(matches!(
            bad_mask,
            super::IndexError::LengthMismatch {
                expected: 3,
                actual: 5,
                ..
            }
        ));
        Ok(())
    }

    #[test]
    fn timedelta_index_where_putmask_match_pandas_nwqty() -> Result<(), super::IndexError> {
        let nat = fp_types::Timedelta::NAT;
        let td = super::TimedeltaIndex::new(vec![100_i64, 200, 300]).set_name("d");

        let masked = td.r#where(&[false, true, false], nat)?;
        assert_eq!(masked.values(), vec![None, Some(200), None]);
        assert_eq!(masked.name(), Some("d"));

        let put = td.putmask(&[false, true, true], 999)?;
        assert_eq!(put.values(), vec![Some(100), Some(999), Some(999)]);

        let bad = td.r#where(&[true, false], nat).unwrap_err();
        assert!(matches!(
            bad,
            super::IndexError::LengthMismatch {
                expected: 3,
                actual: 2,
                ..
            }
        ));
        Ok(())
    }

    #[test]
    fn index_variants_searchsorted_match_pandas_tam73() -> Result<(), super::IndexError> {
        const NS: i64 = 1_000_000_000;
        let a = 1_704_067_200_i64 * NS;
        let b = 1_705_276_800_i64 * NS;
        let c = 1_706_140_800_i64 * NS;
        let dt = super::DatetimeIndex::new(vec![a, b, c]);

        assert_eq!(dt.searchsorted(a, "left")?, 0);
        assert_eq!(dt.searchsorted(a, "right")?, 1);
        assert_eq!(dt.searchsorted(c, "right")?, 3);
        // Mid-range insertion (between a and b).
        let mid = a + 1;
        assert_eq!(dt.searchsorted(mid, "left")?, 1);

        // Bad side.
        assert!(dt.searchsorted(a, "middle").is_err());

        let td = super::TimedeltaIndex::new(vec![100_i64, 200, 300]);
        assert_eq!(td.searchsorted(150, "left")?, 1);
        assert_eq!(td.searchsorted(200, "right")?, 2);

        use fp_types::{Period, PeriodFreq};
        let p1 = Period::new(10, PeriodFreq::Monthly);
        let p2 = Period::new(11, PeriodFreq::Monthly);
        let p3 = Period::new(12, PeriodFreq::Monthly);
        let pi = super::PeriodIndex::new(vec![p1, p2, p3]);
        assert_eq!(pi.searchsorted(p2, "left")?, 1);
        assert_eq!(pi.searchsorted(p3, "right")?, 3);
        // Mismatched freq rejects.
        let mismatch = Period::new(10, PeriodFreq::Annual);
        assert!(pi.searchsorted(mismatch, "left").is_err());

        let r = super::RangeIndex::new(0, 10, 2).unwrap();
        // Range values: [0, 2, 4, 6, 8].
        assert_eq!(r.searchsorted(4, "left")?, 2);
        assert_eq!(r.searchsorted(4, "right")?, 3);
        assert_eq!(r.searchsorted(7, "left")?, 4);

        // Descending range rejects.
        let desc = super::RangeIndex::new(10, 0, -2).unwrap();
        assert!(desc.searchsorted(4, "left").is_err());
        Ok(())
    }

    #[test]
    fn datetime_timedelta_get_indexer_non_unique_match_pandas_sm32a() {
        const NS: i64 = 1_000_000_000;
        let a = 1_704_067_200_i64 * NS;
        let b = 1_705_276_800_i64 * NS;
        // 4-element index with a duplicated `a`.
        let dt = super::DatetimeIndex::new(vec![a, b, a, b]);
        let (positions, missing) = dt.get_indexer_non_unique(&[a, b + 99]);
        // a matches positions 0 and 2; b+99 is missing.
        assert_eq!(positions, vec![0, 2, -1]);
        assert_eq!(missing, vec![1]);

        let td = super::TimedeltaIndex::new(vec![100_i64, 200, 100]);
        let (positions, missing) = td.get_indexer_non_unique(&[100, 999]);
        assert_eq!(positions, vec![0, 2, -1]);
        assert_eq!(missing, vec![1]);
    }

    #[test]
    fn datetime_timedelta_get_loc_get_indexer_match_pandas_6x9de() -> Result<(), super::IndexError>
    {
        const NS: i64 = 1_000_000_000;
        let a = 1_704_067_200_i64 * NS;
        let b = 1_705_276_800_i64 * NS;
        let c = 1_706_140_800_i64 * NS;
        let dt = super::DatetimeIndex::new(vec![a, b, c]);

        // get_loc finds first position.
        assert_eq!(dt.get_loc(b)?, 1);
        let missing_err = dt.get_loc(b + 1).unwrap_err();
        assert!(matches!(
            missing_err,
            super::IndexError::InvalidArgument(ref msg) if msg.contains("get_loc")
        ));

        // get_indexer maps each target.
        let mapped = dt.get_indexer(&[c, a, b + 999]);
        assert_eq!(mapped, vec![2, 0, -1]);

        // TimedeltaIndex spot check.
        let td = super::TimedeltaIndex::new(vec![100_i64, 200, 300]);
        assert_eq!(td.get_loc(200)?, 1);
        assert_eq!(td.get_indexer(&[300, 999, 100]), vec![2, -1, 0]);
        Ok(())
    }

    #[test]
    fn datetime_timedelta_slice_indexer_match_pandas_95eqf() -> Result<(), super::IndexError> {
        const NS: i64 = 1_000_000_000;
        let a = 1_704_067_200_i64 * NS;
        let b = 1_705_276_800_i64 * NS;
        let c = 1_706_140_800_i64 * NS;
        let dt = super::DatetimeIndex::new(vec![a, b, c]);
        assert_eq!(dt.slice_indexer(b, c)?, 1..3);
        assert_eq!(dt.slice_indexer(a, c)?, 0..3);

        let td = super::TimedeltaIndex::new(vec![100_i64, 200, 300]);
        assert_eq!(td.slice_indexer(150, 250)?, 1..2);
        Ok(())
    }

    #[test]
    fn datetime_timedelta_get_slice_bound_match_pandas_x7r04() -> Result<(), super::IndexError> {
        const NS: i64 = 1_000_000_000;
        let a = 1_704_067_200_i64 * NS;
        let b = 1_705_276_800_i64 * NS;
        let c = 1_706_140_800_i64 * NS;
        let dt = super::DatetimeIndex::new(vec![a, b, c]);
        assert_eq!(dt.get_slice_bound(b, "left")?, 1);
        assert_eq!(dt.get_slice_bound(b, "right")?, 2);
        assert!(dt.get_slice_bound(b, "middle").is_err());

        let td = super::TimedeltaIndex::new(vec![100_i64, 200, 300]);
        assert_eq!(td.get_slice_bound(150, "left")?, 1);
        assert_eq!(td.get_slice_bound(200, "right")?, 2);
        Ok(())
    }

    #[test]
    fn datetime_timedelta_slice_locs_match_pandas_mxedz() -> Result<(), super::IndexError> {
        const NS: i64 = 1_000_000_000;
        let a = 1_704_067_200_i64 * NS;
        let b = 1_705_276_800_i64 * NS;
        let c = 1_706_140_800_i64 * NS;
        let d = 1_707_350_400_i64 * NS;
        let dt = super::DatetimeIndex::new(vec![a, b, c, d]);

        // Slice [b, c]: positions [1, 3) (right is exclusive in slice_locs).
        assert_eq!(dt.slice_locs(b, c)?, (1, 3));
        // Slice [a, d]: full range.
        assert_eq!(dt.slice_locs(a, d)?, (0, 4));
        // Slice past the end: empty range.
        assert_eq!(dt.slice_locs(d + 1, d + 2)?, (4, 4));

        // Non-monotonic rejects.
        let unsorted = super::DatetimeIndex::new(vec![c, a, b, d]);
        assert!(unsorted.slice_locs(a, c).is_err());

        // TimedeltaIndex spot check.
        let td = super::TimedeltaIndex::new(vec![100_i64, 200, 300]);
        assert_eq!(td.slice_locs(150, 250)?, (1, 2));

        Ok(())
    }

    #[test]
    fn index_variants_to_flat_index_match_pandas_wcpw5() {
        const NS: i64 = 1_000_000_000;
        let dt = super::DatetimeIndex::new(vec![1_704_067_200_i64 * NS]).set_name("ts");
        let dt_flat = dt.to_flat_index();
        assert_eq!(dt_flat.len(), 1);
        assert_eq!(dt_flat.name(), Some("ts"));
        assert!(matches!(
            dt_flat.labels()[0],
            super::IndexLabel::Datetime64(_)
        ));
        assert_eq!(dt.to_frame(), dt_flat.to_frame());
        assert_eq!(dt.to_series(), dt_flat.to_series());

        let td = super::TimedeltaIndex::new(vec![100_i64]).set_name("d");
        let td_flat = td.to_flat_index();
        assert_eq!(td_flat.len(), 1);
        assert_eq!(td_flat.name(), Some("d"));
        assert_eq!(td.to_frame(), td_flat.to_frame());
        assert_eq!(td.to_series(), td_flat.to_series());

        use fp_types::{Period, PeriodFreq};
        let pi = super::PeriodIndex::new(vec![Period::new(10, PeriodFreq::Monthly)]).set_name("p");
        let pi_flat = pi.to_flat_index();
        assert_eq!(pi_flat.len(), 1);
        assert!(matches!(pi_flat.labels()[0], super::IndexLabel::Utf8(_)));
        assert_eq!(pi.to_frame(), pi_flat.to_frame());
        assert_eq!(pi.to_series(), pi_flat.to_series());

        let r = super::RangeIndex::new(0, 3, 1).unwrap().set_name("r");
        let r_flat = r.to_flat_index();
        assert_eq!(r_flat.len(), 3);
        assert_eq!(r_flat.name(), Some("r"));
        assert_eq!(r.to_frame(), r_flat.to_frame());
        assert_eq!(r.to_series(), r_flat.to_series());

        let cat = super::CategoricalIndex::from_values(vec!["a".to_owned(), "b".to_owned()], false);
        let cat_flat = cat.to_flat_index();
        assert_eq!(cat_flat.len(), 2);
        assert_eq!(cat.to_frame(), cat_flat.to_frame());
        assert_eq!(cat.to_series(), cat_flat.to_series());
    }

    #[test]
    fn index_variants_all_any_forward_flat_truthiness_ejwyw() {
        const NS: i64 = 1_000_000_000;

        let dt = super::DatetimeIndex::new(vec![0, NS]);
        let dt_flat = dt.to_flat_index();
        assert_eq!(dt.any(), dt_flat.any());
        assert_eq!(dt.all(), dt_flat.all());
        assert!(dt.any());
        assert!(!dt.all());

        let td = super::TimedeltaIndex::new(vec![0, 5]);
        let td_flat = td.to_flat_index();
        assert_eq!(td.any(), td_flat.any());
        assert_eq!(td.all(), td_flat.all());
        assert!(td.any());
        assert!(!td.all());

        use fp_types::{Period, PeriodFreq};
        let pi = super::PeriodIndex::new(vec![
            Period::new(1, PeriodFreq::Monthly),
            Period::new(2, PeriodFreq::Monthly),
        ]);
        let pi_flat = pi.to_flat_index();
        assert_eq!(pi.any(), pi_flat.any());
        assert_eq!(pi.all(), pi_flat.all());
        assert!(pi.any());
        assert!(pi.all());

        let range = super::RangeIndex::new(0, 3, 1).unwrap();
        let range_flat = range.to_flat_index();
        assert_eq!(range.any(), range_flat.any());
        assert_eq!(range.all(), range_flat.all());
        assert!(range.any());
        assert!(!range.all());

        let empty_range = super::RangeIndex::new(0, 0, 1).unwrap();
        assert!(!empty_range.any());
        assert!(empty_range.all());

        let cat = super::CategoricalIndex::from_values(vec![String::new(), "x".to_owned()], false);
        let cat_flat = cat.to_flat_index();
        assert_eq!(cat.any(), cat_flat.any());
        assert_eq!(cat.all(), cat_flat.all());
        assert!(cat.any());
        assert!(!cat.all());
    }

    #[test]
    fn index_variants_get_level_values_forward_flat_xf0zn() -> Result<(), super::IndexError> {
        const NS: i64 = 1_000_000_000;

        let dt = super::DatetimeIndex::new(vec![NS, 2 * NS]).set_name("ts");
        assert_eq!(dt.get_level_values(0)?, dt.to_flat_index());

        let td = super::TimedeltaIndex::new(vec![5, 10]).set_name("delta");
        assert_eq!(td.get_level_values(0)?, td.to_flat_index());

        use fp_types::{Period, PeriodFreq};
        let pi =
            super::PeriodIndex::new(vec![Period::new(1, PeriodFreq::Monthly)]).set_name("period");
        assert_eq!(pi.get_level_values(0)?, pi.to_flat_index());

        let range = super::RangeIndex::new(1, 4, 1)?.set_name("row");
        assert_eq!(range.get_level_values(0)?, range.to_flat_index());

        let cat =
            super::CategoricalIndex::from_values(vec!["a".to_owned()], false).set_name("category");
        assert_eq!(cat.get_level_values(0)?, cat.to_flat_index());

        assert!(matches!(
            cat.get_level_values(1),
            Err(super::IndexError::OutOfBounds {
                position: 1,
                length: 1
            })
        ));

        Ok(())
    }

    #[test]
    fn index_variants_droplevel_forward_flat_errors_t8vpw() -> Result<(), super::IndexError> {
        const NS: i64 = 1_000_000_000;

        let dt = super::DatetimeIndex::new(vec![NS, 2 * NS]).set_name("ts");
        assert!(matches!(
            dt.droplevel(0),
            Err(super::IndexError::InvalidArgument(message))
                if message == "cannot remove the only level from a flat Index"
        ));

        let td = super::TimedeltaIndex::new(vec![5, 10]).set_name("delta");
        assert!(matches!(
            td.droplevel(0),
            Err(super::IndexError::InvalidArgument(message))
                if message == "cannot remove the only level from a flat Index"
        ));

        use fp_types::{Period, PeriodFreq};
        let pi =
            super::PeriodIndex::new(vec![Period::new(1, PeriodFreq::Monthly)]).set_name("period");
        assert!(matches!(
            pi.droplevel(0),
            Err(super::IndexError::InvalidArgument(message))
                if message == "cannot remove the only level from a flat Index"
        ));

        let range = super::RangeIndex::new(1, 4, 1)?.set_name("row");
        assert!(matches!(
            range.droplevel(0),
            Err(super::IndexError::InvalidArgument(message))
                if message == "cannot remove the only level from a flat Index"
        ));

        let cat =
            super::CategoricalIndex::from_values(vec!["a".to_owned()], false).set_name("category");
        assert!(matches!(
            cat.droplevel(1),
            Err(super::IndexError::OutOfBounds {
                position: 1,
                length: 1
            })
        ));

        Ok(())
    }

    #[test]
    fn index_variants_groupby_forward_flat_buckets_vypi3() {
        const NS: i64 = 1_000_000_000;

        let dt = super::DatetimeIndex::new(vec![NS, 2 * NS, NS]);
        assert_eq!(dt.groupby(), dt.to_flat_index().groupby());

        let td = super::TimedeltaIndex::new(vec![5, 10, 5]);
        assert_eq!(td.groupby(), td.to_flat_index().groupby());

        use fp_types::{Period, PeriodFreq};
        let pi = super::PeriodIndex::new(vec![
            Period::new(1, PeriodFreq::Monthly),
            Period::new(2, PeriodFreq::Monthly),
            Period::new(1, PeriodFreq::Monthly),
        ]);
        assert_eq!(pi.groupby(), pi.to_flat_index().groupby());

        let range = super::RangeIndex::new(2, 8, 2).unwrap();
        assert_eq!(range.groupby(), range.to_flat_index().groupby());

        let cat = super::CategoricalIndex::from_values(
            vec!["a".to_owned(), "b".to_owned(), "a".to_owned()],
            false,
        );
        assert_eq!(cat.groupby(), cat.to_flat_index().groupby());
        assert_eq!(
            cat.groupby()
                .get(&super::IndexLabel::Utf8("a".to_owned()))
                .cloned(),
            Some(vec![0, 2])
        );
    }

    #[test]
    fn index_variants_map_forward_flat_and_preserve_name_vxlfs() {
        const NS: i64 = 1_000_000_000;

        let dt = super::DatetimeIndex::new(vec![NS, 2 * NS]).set_name("ts");
        let dt_mapped = dt.map(|label| match label {
            super::IndexLabel::Datetime64(nanos) => super::IndexLabel::Int64(*nanos / NS),
            other => other.clone(),
        });
        assert_eq!(
            dt_mapped.labels(),
            &[super::IndexLabel::Int64(1), super::IndexLabel::Int64(2)]
        );
        assert_eq!(dt_mapped.name(), Some("ts"));

        let td = super::TimedeltaIndex::new(vec![5, 10]).set_name("delta");
        assert_eq!(
            td.map(|label| match label {
                super::IndexLabel::Timedelta64(nanos) => super::IndexLabel::Int64(*nanos * 2),
                other => other.clone(),
            }),
            td.to_flat_index().map(|label| match label {
                super::IndexLabel::Timedelta64(nanos) => super::IndexLabel::Int64(*nanos * 2),
                other => other.clone(),
            })
        );

        use fp_types::{Period, PeriodFreq};
        let pi = super::PeriodIndex::new(vec![Period::new(1, PeriodFreq::Monthly)]);
        assert_eq!(
            pi.map(|label| super::IndexLabel::Utf8(format!("p:{label}"))),
            pi.to_flat_index()
                .map(|label| super::IndexLabel::Utf8(format!("p:{label}")))
        );

        let range = super::RangeIndex::new(1, 4, 1).unwrap();
        assert_eq!(
            range.map(|label| match label {
                super::IndexLabel::Int64(v) => super::IndexLabel::Int64(*v + 10),
                other => other.clone(),
            }),
            range.to_flat_index().map(|label| match label {
                super::IndexLabel::Int64(v) => super::IndexLabel::Int64(*v + 10),
                other => other.clone(),
            })
        );

        let cat = super::CategoricalIndex::from_values(vec!["a".to_owned()], false);
        assert_eq!(
            cat.map(|label| super::IndexLabel::Utf8(label.to_string().to_uppercase())),
            cat.to_flat_index()
                .map(|label| super::IndexLabel::Utf8(label.to_string().to_uppercase()))
        );
    }

    #[test]
    fn index_variants_astype_forward_flat_and_preserve_name_o5pyg() {
        const NS: i64 = 1_000_000_000;

        let dt = super::DatetimeIndex::new(vec![NS, 2 * NS]).set_name("ts");
        assert_eq!(
            dt.astype("int64").unwrap(),
            dt.to_flat_index().astype("int64").unwrap()
        );
        assert_eq!(dt.astype("int64").unwrap().name(), Some("ts"));
        assert!(dt.astype("float64").is_err());

        let td = super::TimedeltaIndex::new(vec![5, 10]).set_name("delta");
        assert_eq!(
            td.astype("string").unwrap(),
            td.to_flat_index().astype("string").unwrap()
        );
        assert_eq!(td.astype("string").unwrap().name(), Some("delta"));

        use fp_types::{Period, PeriodFreq};
        let pi = super::PeriodIndex::new(vec![Period::new(1, PeriodFreq::Monthly)]);
        assert_eq!(
            pi.astype("object").unwrap(),
            pi.to_flat_index().astype("object").unwrap()
        );

        let range = super::RangeIndex::new(1, 4, 1).unwrap().set_name("r");
        assert_eq!(
            range.astype("str").unwrap(),
            range.to_flat_index().astype("str").unwrap()
        );
        assert_eq!(range.astype("str").unwrap().name(), Some("r"));

        let cat = super::CategoricalIndex::from_values(vec!["7".to_owned()], false);
        assert_eq!(
            cat.astype("int").unwrap(),
            cat.to_flat_index().astype("int").unwrap()
        );
        assert!(cat.astype("datetime64[ns]").is_err());
    }

    #[test]
    fn index_variants_asof_forward_flat_and_mask_locs_955dj() {
        const NS: i64 = 1_000_000_000;

        let dt = super::DatetimeIndex::new(vec![NS, 3 * NS, 5 * NS]);
        let dt_key = super::IndexLabel::Datetime64(4 * NS);
        assert_eq!(dt.asof(&dt_key), dt.to_flat_index().asof(&dt_key));
        assert_eq!(dt.asof(&super::IndexLabel::Datetime64(0)), None);

        let td = super::TimedeltaIndex::new(vec![10, 20, 30]);
        let where_td = super::Index::new(vec![
            super::IndexLabel::Timedelta64(5),
            super::IndexLabel::Timedelta64(20),
            super::IndexLabel::Timedelta64(25),
        ]);
        let mask = [false, true, true];
        assert_eq!(
            td.asof_locs(&where_td, Some(&mask)),
            td.to_flat_index().asof_locs(&where_td, Some(&mask))
        );
        assert_eq!(
            td.asof_locs(&where_td, Some(&mask)),
            vec![None, Some(1), Some(1)]
        );

        use fp_types::{Period, PeriodFreq};
        let pi = super::PeriodIndex::new(vec![
            Period::new(1, PeriodFreq::Monthly),
            Period::new(2, PeriodFreq::Monthly),
        ]);
        let period_key = pi.to_flat_index().labels()[1].clone();
        assert_eq!(pi.asof(&period_key), pi.to_flat_index().asof(&period_key));

        let range = super::RangeIndex::new(2, 8, 2).unwrap();
        let range_key = super::IndexLabel::Int64(5);
        assert_eq!(
            range.asof(&range_key),
            range.to_flat_index().asof(&range_key)
        );

        let cat = super::CategoricalIndex::from_values(
            vec!["a".to_owned(), "c".to_owned(), "e".to_owned()],
            false,
        );
        let cat_key = super::IndexLabel::Utf8("d".to_owned());
        assert_eq!(cat.asof(&cat_key), cat.to_flat_index().asof(&cat_key));
    }

    #[test]
    fn index_variants_drop_join_sortlevel_forward_flat_gr6kj() {
        const NS: i64 = 1_000_000_000;

        let dt = super::DatetimeIndex::new(vec![NS, 3 * NS, 5 * NS]).set_name("ts");
        let drop_dt = [super::IndexLabel::Datetime64(3 * NS)];
        assert_eq!(dt.drop(&drop_dt), dt.to_flat_index().drop(&drop_dt));
        assert_eq!(dt.drop(&drop_dt).name(), Some("ts"));

        let td = super::TimedeltaIndex::new(vec![30, 10, 20]);
        let (td_sorted, td_order) = td.sortlevel();
        let (flat_td_sorted, flat_td_order) = td.to_flat_index().sortlevel();
        assert_eq!(td_sorted, flat_td_sorted);
        assert_eq!(td_order, flat_td_order);

        use fp_types::{Period, PeriodFreq};
        let pi = super::PeriodIndex::new(vec![
            Period::new(2, PeriodFreq::Monthly),
            Period::new(1, PeriodFreq::Monthly),
        ]);
        assert_eq!(
            pi.join(&pi.to_flat_index(), "outer").unwrap(),
            pi.to_flat_index()
        );

        let range = super::RangeIndex::new(2, 8, 2).unwrap();
        let other = super::Index::new(vec![
            super::IndexLabel::Int64(4),
            super::IndexLabel::Int64(6),
            super::IndexLabel::Int64(9),
        ]);
        assert_eq!(
            range.join(&other, "inner").unwrap(),
            range.to_flat_index().join(&other, "inner").unwrap()
        );
        assert!(range.join(&other, "sideways").is_err());

        let cat = super::CategoricalIndex::from_values(
            vec!["b".to_owned(), "a".to_owned(), "b".to_owned()],
            false,
        );
        let (cat_sorted, cat_order) = cat.sortlevel();
        let (flat_cat_sorted, flat_cat_order) = cat.to_flat_index().sortlevel();
        assert_eq!(cat_sorted, flat_cat_sorted);
        assert_eq!(cat_order, flat_cat_order);
        let drop_cat = [super::IndexLabel::Utf8("b".to_owned())];
        assert_eq!(cat.drop(&drop_cat), cat.to_flat_index().drop(&drop_cat));
    }

    #[test]
    fn index_variants_temporal_rounding_forwarders_dznxu() {
        let hour = fp_types::Timedelta::NANOS_PER_HOUR;
        let minute = fp_types::Timedelta::NANOS_PER_MIN;
        let nat = fp_types::Timedelta::NAT;
        let dt =
            super::DatetimeIndex::new(vec![hour / 2, hour + 31 * minute, i64::MIN]).set_name("ts");

        let dt_floor = dt.floor("h").unwrap();
        assert_eq!(dt_floor.asi8(), vec![0, hour, i64::MIN]);
        assert_eq!(dt_floor.name(), Some("ts"));

        let dt_ceil = dt.ceil("h").unwrap();
        assert_eq!(dt_ceil.asi8(), vec![hour, 2 * hour, i64::MIN]);

        let dt_round = dt.round("h").unwrap();
        assert_eq!(dt_round.asi8(), vec![0, 2 * hour, i64::MIN]);

        let dt_snap = dt.snap("h").unwrap();
        assert_eq!(dt_snap.asi8(), dt.asi8());
        assert!(dt.floor("not-a-frequency").is_err());
        assert!(dt.snap("not-a-frequency").is_err());

        let td = super::TimedeltaIndex::new(vec![hour / 2, hour + 31 * minute, nat]).set_name("d");
        assert_eq!(td.floor("h").unwrap().asi8(), vec![0, hour, nat]);
        assert_eq!(td.ceil("h").unwrap().asi8(), vec![hour, 2 * hour, nat]);
        assert_eq!(td.round("h").unwrap().asi8(), vec![0, 2 * hour, nat]);
        assert_eq!(td.round("h").unwrap().name(), Some("d"));
        assert!(td.ceil("not-a-frequency").is_err());

        use fp_types::{Period, PeriodFreq};
        let periods = super::PeriodIndex::new(vec![
            Period::new(10, PeriodFreq::Monthly),
            Period::new(11, PeriodFreq::Monthly),
        ])
        .set_name("p");
        let rounded_periods = periods.round("not-a-frequency");
        assert_eq!(rounded_periods.values(), periods.values());
        assert_eq!(rounded_periods.name(), Some("p"));
    }

    #[test]
    fn index_variants_diff_forwarders_lqs0a() {
        let day = fp_types::Timedelta::NANOS_PER_DAY;
        let nat = fp_types::Timedelta::NAT;

        let dt = super::DatetimeIndex::new(vec![day, 3 * day, i64::MIN, 10 * day]).set_name("ts");
        assert_eq!(dt.diff(1).asi8(), vec![nat, 2 * day, nat, nat]);
        assert_eq!(dt.diff(-1).asi8(), vec![-2 * day, nat, nat, nat]);
        assert_eq!(dt.diff(0).asi8(), vec![0, 0, nat, 0]);
        assert_eq!(dt.diff(1).name(), Some("ts"));

        let td = super::TimedeltaIndex::new(vec![day, 4 * day, nat, 9 * day]).set_name("delta");
        assert_eq!(td.diff(2).asi8(), vec![nat, nat, nat, 5 * day]);
        assert_eq!(td.diff(-1).asi8(), vec![-3 * day, nat, nat, nat]);
        assert_eq!(td.diff(0).asi8(), vec![0, 0, nat, 0]);
        assert_eq!(td.diff(1).name(), Some("delta"));

        use fp_types::{Period, PeriodFreq};
        let periods = super::PeriodIndex::new(vec![
            Period::new(10, PeriodFreq::Monthly),
            Period::new(12, PeriodFreq::Monthly),
            Period::new(13, PeriodFreq::Quarterly),
            Period::new(15, PeriodFreq::Quarterly),
        ]);
        assert_eq!(periods.diff(1), vec![None, Some(2), None, Some(2)]);
        assert_eq!(periods.diff(-1), vec![Some(-2), None, Some(-2), None]);
        assert_eq!(periods.diff(0), vec![Some(0), Some(0), Some(0), Some(0)]);

        let range = super::RangeIndex::new(2, 10, 2).unwrap().set_name("r");
        assert_eq!(range.diff(1), vec![None, Some(2), Some(2), Some(2)]);
        assert_eq!(range.diff(-2), vec![Some(-4), Some(-4), None, None]);
        assert_eq!(range.diff(0), vec![Some(0), Some(0), Some(0), Some(0)]);
        assert_eq!(range.name(), Some("r"));

        let cat = super::CategoricalIndex::from_values(vec!["a".to_owned(), "b".to_owned()], false);
        let err = cat.diff(1).unwrap_err();
        assert!(matches!(
            err,
            super::IndexError::InvalidArgument(message)
                if message.contains("Categorical has no 'diff' method")
        ));
    }

    #[test]
    fn datetime_index_to_period_matches_pandas_ordinals_002sq()
    -> Result<(), Box<dyn std::error::Error>> {
        fn ns(value: &str) -> Result<i64, super::DateRangeError> {
            super::parse_datetime_to_nanos(value)
        }

        use fp_types::{Period, PeriodFreq};

        let dt = super::DatetimeIndex::new(vec![
            ns("1969-12-31 23:59:59")?,
            ns("1970-01-01 00:00:00")?,
            ns("2024-02-29 12:34:56")?,
        ])
        .set_name("ts");

        assert_eq!(
            dt.to_period("Y")?.values(),
            &[
                Period::new(-1, PeriodFreq::Annual),
                Period::new(0, PeriodFreq::Annual),
                Period::new(54, PeriodFreq::Annual),
            ]
        );
        assert_eq!(
            dt.to_period("Q")?.values(),
            &[
                Period::new(-1, PeriodFreq::Quarterly),
                Period::new(0, PeriodFreq::Quarterly),
                Period::new(216, PeriodFreq::Quarterly),
            ]
        );
        assert_eq!(
            dt.to_period("M")?.values(),
            &[
                Period::new(-1, PeriodFreq::Monthly),
                Period::new(0, PeriodFreq::Monthly),
                Period::new(649, PeriodFreq::Monthly),
            ]
        );
        assert_eq!(
            dt.to_period("D")?.values(),
            &[
                Period::new(-1, PeriodFreq::Daily),
                Period::new(0, PeriodFreq::Daily),
                Period::new(19_782, PeriodFreq::Daily),
            ]
        );
        assert_eq!(
            dt.to_period("W")?.values(),
            &[
                Period::new(1, PeriodFreq::Weekly),
                Period::new(1, PeriodFreq::Weekly),
                Period::new(2_827, PeriodFreq::Weekly),
            ]
        );
        assert_eq!(
            dt.to_period("B")?.values(),
            &[
                Period::new(-1, PeriodFreq::Business),
                Period::new(0, PeriodFreq::Business),
                Period::new(14_130, PeriodFreq::Business),
            ]
        );
        assert_eq!(
            dt.to_period("H")?.values(),
            &[
                Period::new(-1, PeriodFreq::Hourly),
                Period::new(0, PeriodFreq::Hourly),
                Period::new(474_780, PeriodFreq::Hourly),
            ]
        );
        let minutely = dt.to_period("min")?;
        assert_eq!(
            minutely.values(),
            &[
                Period::new(-1, PeriodFreq::Minutely),
                Period::new(0, PeriodFreq::Minutely),
                Period::new(28_486_834, PeriodFreq::Minutely),
            ]
        );
        assert_eq!(minutely.name(), Some("ts"));
        assert_eq!(
            dt.to_period("S")?.values(),
            &[
                Period::new(-1, PeriodFreq::Secondly),
                Period::new(0, PeriodFreq::Secondly),
                Period::new(1_709_210_096, PeriodFreq::Secondly),
            ]
        );

        assert!(matches!(
            super::DatetimeIndex::new(vec![i64::MIN]).to_period("M"),
            Err(super::IndexError::InvalidArgument(message))
                if message.contains("invalid or NaT datetime nanos")
        ));
        assert!(matches!(
            dt.to_period("fortnight"),
            Err(super::IndexError::InvalidArgument(message))
                if message.contains("unsupported frequency")
        ));

        Ok(())
    }

    #[test]
    fn period_index_asfreq_boundary_conversion_h1zia() -> Result<(), super::IndexError> {
        use fp_types::{Period, PeriodFreq};

        let annual = super::PeriodIndex::new(vec![
            Period::new(0, PeriodFreq::Annual),
            Period::new(1, PeriodFreq::Annual),
        ])
        .set_name("p");
        assert_eq!(
            annual.asfreq("M")?.values(),
            &[
                Period::new(11, PeriodFreq::Monthly),
                Period::new(23, PeriodFreq::Monthly),
            ]
        );
        let annual_start = annual.asfreq_with_how("M", "start")?;
        assert_eq!(
            annual_start.values(),
            &[
                Period::new(0, PeriodFreq::Monthly),
                Period::new(12, PeriodFreq::Monthly),
            ]
        );
        assert_eq!(annual_start.name(), Some("p"));

        let quarterly = super::PeriodIndex::new(vec![
            Period::new(0, PeriodFreq::Quarterly),
            Period::new(1, PeriodFreq::Quarterly),
        ]);
        assert_eq!(
            quarterly.asfreq("D")?.values(),
            &[
                Period::new(89, PeriodFreq::Daily),
                Period::new(180, PeriodFreq::Daily),
            ]
        );
        assert_eq!(
            quarterly.asfreq_with_how("D", "s")?.values(),
            &[
                Period::new(0, PeriodFreq::Daily),
                Period::new(90, PeriodFreq::Daily),
            ]
        );

        let monthly = super::PeriodIndex::new(vec![
            Period::new(0, PeriodFreq::Monthly),
            Period::new(1, PeriodFreq::Monthly),
        ]);
        assert_eq!(
            monthly.asfreq("S")?.values(),
            &[
                Period::new(2_678_399, PeriodFreq::Secondly),
                Period::new(5_097_599, PeriodFreq::Secondly),
            ]
        );
        assert_eq!(
            monthly.asfreq_with_how("S", "begin")?.values(),
            &[
                Period::new(0, PeriodFreq::Secondly),
                Period::new(2_678_400, PeriodFreq::Secondly),
            ]
        );
        assert_eq!(
            monthly.asfreq("B")?.values(),
            &[
                Period::new(21, PeriodFreq::Business),
                Period::new(41, PeriodFreq::Business),
            ]
        );
        assert_eq!(
            monthly.asfreq_with_how("W", "start")?.values(),
            &[
                Period::new(1, PeriodFreq::Weekly),
                Period::new(5, PeriodFreq::Weekly),
            ]
        );
        assert!(matches!(
            monthly.asfreq_with_how("D", "middle"),
            Err(super::IndexError::InvalidArgument(message))
                if message.contains("asfreq how must be 'start' or 'end'")
        ));
        assert!(matches!(
            monthly.asfreq("fortnight"),
            Err(super::IndexError::InvalidArgument(message))
                if message.contains("unsupported frequency")
        ));

        Ok(())
    }

    #[test]
    fn period_index_timestamp_boundaries_d44wh() -> Result<(), Box<dyn std::error::Error>> {
        fn ns(value: &str) -> Result<i64, super::DateRangeError> {
            super::parse_datetime_to_nanos(value)
        }

        use fp_types::{Period, PeriodFreq};

        let monthly = super::PeriodIndex::new(vec![
            Period::new(0, PeriodFreq::Monthly),
            Period::new(1, PeriodFreq::Monthly),
        ])
        .set_name("period");
        assert_eq!(
            monthly.start_time()?.asi8(),
            vec![ns("1970-01-01 00:00:00")?, ns("1970-02-01 00:00:00")?]
        );
        assert_eq!(
            monthly.end_time()?.asi8(),
            vec![
                ns("1970-02-01 00:00:00")? - 1,
                ns("1970-03-01 00:00:00")? - 1
            ]
        );
        assert_eq!(
            monthly.to_timestamp("start")?.asi8(),
            monthly.start_time()?.asi8()
        );
        assert_eq!(
            monthly.to_timestamp("end")?.asi8(),
            monthly.end_time()?.asi8()
        );
        assert_eq!(monthly.to_timestamp("")?.name(), Some("period"));
        assert_eq!(monthly.qyear()?, vec![1970, 1970]);
        assert!(matches!(
            monthly.to_timestamp("middle"),
            Err(super::IndexError::InvalidArgument(message))
                if message.contains("to_timestamp how must be 'start' or 'end'")
        ));

        let quarterly = super::PeriodIndex::new(vec![
            Period::new(-1, PeriodFreq::Quarterly),
            Period::new(0, PeriodFreq::Quarterly),
        ]);
        assert_eq!(
            quarterly.start_time()?.asi8(),
            vec![ns("1969-10-01 00:00:00")?, ns("1970-01-01 00:00:00")?]
        );
        assert_eq!(
            quarterly.end_time()?.asi8(),
            vec![
                ns("1970-01-01 00:00:00")? - 1,
                ns("1970-04-01 00:00:00")? - 1
            ]
        );
        assert_eq!(quarterly.qyear()?, vec![1969, 1970]);

        let mixed_freq = super::PeriodIndex::new(vec![
            Period::new(1, PeriodFreq::Weekly),
            Period::new(2, PeriodFreq::Business),
            Period::new(1, PeriodFreq::Hourly),
        ]);
        assert_eq!(
            mixed_freq.start_time()?.asi8(),
            vec![
                ns("1969-12-29 00:00:00")?,
                ns("1970-01-05 00:00:00")?,
                fp_types::Timedelta::NANOS_PER_HOUR
            ]
        );
        assert_eq!(
            mixed_freq.end_time()?.asi8(),
            vec![
                ns("1970-01-05 00:00:00")? - 1,
                ns("1970-01-06 00:00:00")? - 1,
                2 * fp_types::Timedelta::NANOS_PER_HOUR - 1
            ]
        );
        assert_eq!(mixed_freq.qyear()?, vec![1970, 1970, 1970]);

        Ok(())
    }

    #[test]
    fn index_variants_view_transpose_ravel_nlevels_infer_objects_match_pandas_d0ph1() {
        const NS: i64 = 1_000_000_000;
        let dt = super::DatetimeIndex::new(vec![1_704_067_200_i64 * NS, i64::MIN]).set_name("ts");
        assert!(dt.view().equals(&dt));
        assert!(dt.transpose().equals(&dt));
        assert!(dt.T().identical(&dt));
        assert_eq!(dt.ravel(), dt.values());
        assert_eq!(dt.nlevels(), 1);
        assert!(dt.infer_objects().equals(&dt));

        let td = super::TimedeltaIndex::new(vec![100_i64, fp_types::Timedelta::NAT]).set_name("d");
        assert!(td.view().equals(&td));
        assert!(td.T().identical(&td));
        assert_eq!(td.ravel(), td.values());
        assert_eq!(td.nlevels(), 1);

        use fp_types::{Period, PeriodFreq};
        let pi = super::PeriodIndex::new(vec![
            Period::new(10, PeriodFreq::Monthly),
            Period::new(11, PeriodFreq::Monthly),
        ]);
        assert_eq!(pi.view().values(), pi.values());
        assert!(pi.T().identical(&pi));
        assert_eq!(pi.ravel(), pi.values().to_vec());
        assert_eq!(pi.nlevels(), 1);

        let r = super::RangeIndex::new(0, 5, 1).unwrap();
        assert!(r.view().equals(&r));
        assert!(r.T().identical(&r));
        assert_eq!(r.ravel(), r.values());
        assert_eq!(r.nlevels(), 1);

        let cat = super::CategoricalIndex::from_values(vec!["a".to_owned(), "b".to_owned()], false);
        assert_eq!(cat.view().labels(), cat.labels());
        assert!(cat.T().identical(&cat));
        assert_eq!(cat.ravel(), cat.labels().to_vec());
        assert_eq!(cat.nlevels(), 1);
    }

    #[test]
    fn datetime_index_set_ops_match_pandas_ik8if() {
        const NS: i64 = 1_000_000_000;
        let a = 1_704_067_200_i64 * NS;
        let b = 1_705_276_800_i64 * NS;
        let c = 1_706_140_800_i64 * NS;
        let d = 1_707_350_400_i64 * NS;
        let left = super::DatetimeIndex::new(vec![a, b, c]).set_name("ts");
        let right = super::DatetimeIndex::new(vec![b, c, d]).set_name("ts");

        // intersection: b, c (in self order).
        let inter = left.intersection(&right);
        assert_eq!(inter.values(), vec![Some(b), Some(c)]);
        assert_eq!(inter.name(), Some("ts"));

        // union: a, b, c then d.
        let union = left.union(&right);
        assert_eq!(union.values(), vec![Some(a), Some(b), Some(c), Some(d)]);

        // difference: a (only in self).
        let diff = left.difference(&right);
        assert_eq!(diff.values(), vec![Some(a)]);

        // symmetric_difference: a (self-only) then d (other-only).
        let sym = left.symmetric_difference(&right);
        assert_eq!(sym.values(), vec![Some(a), Some(d)]);

        // Mismatched names drop the name.
        let mismatched = super::DatetimeIndex::new(vec![b]).set_name("other");
        assert_eq!(left.intersection(&mismatched).name(), None);
        assert_eq!(left.union(&mismatched).name(), None);
    }

    #[test]
    fn timedelta_index_set_ops_match_pandas_ik8if() {
        let left = super::TimedeltaIndex::new(vec![100_i64, 200, 300]).set_name("d");
        let right = super::TimedeltaIndex::new(vec![200_i64, 300, 400]).set_name("d");

        let inter = left.intersection(&right);
        assert_eq!(inter.values(), vec![Some(200), Some(300)]);
        assert_eq!(inter.name(), Some("d"));

        let union = left.union(&right);
        assert_eq!(
            union.values(),
            vec![Some(100), Some(200), Some(300), Some(400)]
        );

        let diff = left.difference(&right);
        assert_eq!(diff.values(), vec![Some(100)]);

        let sym = left.symmetric_difference(&right);
        assert_eq!(sym.values(), vec![Some(100), Some(400)]);
    }

    #[test]
    fn timedelta_index_sum_match_pandas_qi04e() {
        let nat = fp_types::Timedelta::NAT;
        let td = super::TimedeltaIndex::new(vec![10_i64, 20, 30, nat]);
        assert_eq!(td.sum(), Some(60));

        let only_nat = super::TimedeltaIndex::new(vec![nat, nat]);
        assert_eq!(only_nat.sum(), Some(0));

        let empty = super::TimedeltaIndex::new(vec![]);
        assert_eq!(empty.sum(), Some(0));
    }

    #[test]
    fn datetime_timedelta_var_match_pandas_pw5sn() {
        // [10, 20, 30] sample variance with ddof=1: ((100 + 0 + 100)/2) = 100.
        let td = super::TimedeltaIndex::new(vec![10_i64, 20, 30]);
        assert!((td.var().unwrap() - 100.0).abs() < 1e-9);

        // Single element: not enough data.
        let one = super::TimedeltaIndex::new(vec![5_i64]);
        assert_eq!(one.var(), None);

        // DatetimeIndex spot check.
        const NS: i64 = 1_000_000_000;
        let dt = super::DatetimeIndex::new(vec![10 * NS, 20 * NS, 30 * NS]);
        assert!(dt.var().is_some());
    }

    #[test]
    fn datetime_timedelta_std_match_pandas_3hb3t() {
        // [10, 20, 30] sample std with ddof=1: sqrt((100 + 0 + 100) / 2) = 10.
        let td = super::TimedeltaIndex::new(vec![10_i64, 20, 30]);
        assert_eq!(td.std(), Some(10));

        // [10, 30] sample std: sqrt(((10-20)^2 + (30-20)^2) / 1) = sqrt(200).
        let td2 = super::TimedeltaIndex::new(vec![10_i64, 30]);
        let expected = 200f64.sqrt() as i64;
        assert_eq!(td2.std(), Some(expected));

        // Single element / NAT-only: not enough data.
        let one = super::TimedeltaIndex::new(vec![5_i64]);
        assert_eq!(one.std(), None);
        let nat = super::TimedeltaIndex::new(vec![fp_types::Timedelta::NAT]);
        assert_eq!(nat.std(), None);

        // DatetimeIndex spot check.
        const NS: i64 = 1_000_000_000;
        let dt = super::DatetimeIndex::new(vec![10 * NS, 20 * NS, 30 * NS]);
        assert!(dt.std().is_some());
    }

    #[test]
    fn datetime_timedelta_shift_match_pandas_1y3sx() {
        const NS: i64 = 1_000_000_000;
        let day_ns = 86_400 * NS;
        let dt = super::DatetimeIndex::new(vec![1_704_067_200_i64 * NS, i64::MIN]).set_name("ts");

        // Shift by 2 days.
        let shifted = dt.shift(2, day_ns);
        assert_eq!(
            shifted.values()[0],
            Some(1_704_067_200_i64 * NS + 2 * day_ns)
        );
        assert_eq!(shifted.values()[1], None);
        assert_eq!(shifted.name(), Some("ts"));

        // Negative shift.
        let back = dt.shift(-1, day_ns);
        assert_eq!(back.values()[0], Some(1_704_067_200_i64 * NS - day_ns));

        // TimedeltaIndex spot check.
        let td = super::TimedeltaIndex::new(vec![100_i64, fp_types::Timedelta::NAT]);
        let shifted_td = td.shift(3, 50);
        assert_eq!(shifted_td.values()[0], Some(250));
        assert_eq!(shifted_td.values()[1], None);
    }

    #[test]
    fn datetime_timedelta_mean_median_match_pandas_wp0gr() {
        const NS: i64 = 1_000_000_000;
        let a = 1_000_000_000_i64 * NS;
        let b = 2_000_000_000_i64 * NS;
        let c = 3_000_000_000_i64 * NS;
        let dt = super::DatetimeIndex::new(vec![a, b, c, i64::MIN]);
        // Mean: (a + b + c) / 3 = b (the middle, since arithmetic).
        assert_eq!(dt.mean(), Some(b));
        // Median: middle of three sorted values = b.
        assert_eq!(dt.median(), Some(b));

        // Even-length set: median is average of two middles.
        let dt_even = super::DatetimeIndex::new(vec![a, b]);
        let total = i128::from(a) + i128::from(b);
        let expected = i64::try_from(total / 2).unwrap();
        assert_eq!(dt_even.median(), Some(expected));

        // All-NAT.
        let nat = super::DatetimeIndex::new(vec![i64::MIN; 3]);
        assert_eq!(nat.mean(), None);
        assert_eq!(nat.median(), None);

        // Timedelta spot check.
        let td = super::TimedeltaIndex::new(vec![10_i64, 20, 30]);
        assert_eq!(td.mean(), Some(20));
        assert_eq!(td.median(), Some(20));
    }

    #[test]
    fn datetime_index_min_max_sort_values_match_pandas_kastf() {
        const NS: i64 = 1_000_000_000;
        let a = 1_704_067_200_i64 * NS;
        let b = 1_705_276_800_i64 * NS;
        let c = 1_706_140_800_i64 * NS;
        let dt = super::DatetimeIndex::new(vec![b, c, i64::MIN, a]).set_name("ts");

        assert_eq!(dt.min(), Some(a));
        assert_eq!(dt.max(), Some(c));

        let sorted = dt.sort_values();
        let sorted_alias = dt.sort();
        // NAT sorts first (na_position='first' default).
        assert_eq!(sorted.values(), vec![None, Some(a), Some(b), Some(c)]);
        assert_eq!(sorted_alias.values(), sorted.values());
        assert_eq!(sorted.name(), Some("ts"));
        assert_eq!(sorted_alias.name(), Some("ts"));

        let all_nat = super::DatetimeIndex::new(vec![i64::MIN, i64::MIN]);
        assert_eq!(all_nat.min(), None);
        assert_eq!(all_nat.max(), None);

        let empty = super::DatetimeIndex::new(vec![]);
        assert_eq!(empty.min(), None);
        assert_eq!(empty.max(), None);
        assert!(empty.sort_values().is_empty());
        assert!(empty.sort().is_empty());
    }

    #[test]
    fn timedelta_index_min_max_sort_values_match_pandas_kastf() {
        let nat = fp_types::Timedelta::NAT;
        let td = super::TimedeltaIndex::new(vec![300_i64, nat, 100, 200]).set_name("d");

        assert_eq!(td.min(), Some(100));
        assert_eq!(td.max(), Some(300));

        let sorted = td.sort_values();
        let sorted_alias = td.sort();
        assert_eq!(sorted.values(), vec![None, Some(100), Some(200), Some(300)]);
        assert_eq!(sorted_alias.values(), sorted.values());
        assert_eq!(sorted.name(), Some("d"));
        assert_eq!(sorted_alias.name(), Some("d"));

        let all_nat = super::TimedeltaIndex::new(vec![nat, nat]);
        assert_eq!(all_nat.min(), None);
        assert_eq!(all_nat.max(), None);

        let empty = super::TimedeltaIndex::new(vec![]);
        assert_eq!(empty.min(), None);
        assert_eq!(empty.max(), None);
        assert!(empty.sort().is_empty());
    }

    #[test]
    fn datetime_index_append_delete_match_pandas_834v9() -> Result<(), super::IndexError> {
        const NS: i64 = 1_000_000_000;
        let a = 1_704_067_200_i64 * NS;
        let b = 1_705_276_800_i64 * NS;
        let c = 1_706_140_800_i64 * NS;
        let left = super::DatetimeIndex::new(vec![a, b]).set_name("ts");
        let right = super::DatetimeIndex::new(vec![c]).set_name("ts");

        let merged = left.append(&right);
        assert_eq!(merged.values(), vec![Some(a), Some(b), Some(c)]);
        assert_eq!(merged.name(), Some("ts"));

        let mismatched = super::DatetimeIndex::new(vec![c]).set_name("other");
        assert_eq!(left.append(&mismatched).name(), None);

        let trimmed = left.append(&right).delete(1)?;
        assert_eq!(trimmed.values(), vec![Some(a), Some(c)]);
        assert_eq!(trimmed.name(), Some("ts"));

        let oob = left.delete(5).unwrap_err();
        assert!(matches!(
            oob,
            super::IndexError::OutOfBounds {
                position: 5,
                length: 2
            }
        ));
        Ok(())
    }

    #[test]
    fn timedelta_index_append_delete_match_pandas_834v9() -> Result<(), super::IndexError> {
        let left = super::TimedeltaIndex::new(vec![1_i64, 2]).set_name("d");
        let right = super::TimedeltaIndex::new(vec![3_i64]).set_name("d");
        let merged = left.append(&right);
        assert_eq!(merged.values(), vec![Some(1), Some(2), Some(3)]);
        assert_eq!(merged.name(), Some("d"));

        let trimmed = merged.delete(0)?;
        assert_eq!(trimmed.values(), vec![Some(2), Some(3)]);

        assert!(matches!(
            left.delete(7).unwrap_err(),
            super::IndexError::OutOfBounds {
                position: 7,
                length: 2
            }
        ));
        Ok(())
    }

    #[test]
    fn period_index_append_delete_match_pandas_834v9() -> Result<(), super::IndexError> {
        use fp_types::{Period, PeriodFreq};
        let p1 = Period::new(10, PeriodFreq::Monthly);
        let p2 = Period::new(11, PeriodFreq::Monthly);
        let p3 = Period::new(12, PeriodFreq::Monthly);
        let left = super::PeriodIndex::new(vec![p1, p2]).set_name("p");
        let right = super::PeriodIndex::new(vec![p3]).set_name("p");

        let merged = left.append(&right);
        assert_eq!(merged.values(), &[p1, p2, p3]);
        assert_eq!(merged.name(), Some("p"));

        let mismatched = super::PeriodIndex::new(vec![p3]).set_name("other");
        assert_eq!(left.append(&mismatched).name(), None);

        let trimmed = merged.delete(1)?;
        assert_eq!(trimmed.values(), &[p1, p3]);

        assert!(matches!(
            left.delete(5).unwrap_err(),
            super::IndexError::OutOfBounds {
                position: 5,
                length: 2
            }
        ));
        Ok(())
    }

    #[test]
    fn range_index_append_delete_match_pandas_834v9() -> Result<(), super::IndexError> {
        let left = super::RangeIndex::new(0, 3, 1).unwrap();
        let right = super::RangeIndex::new(10, 12, 1).unwrap();
        let merged = left.append(&right);
        let merged_labels = int64_labels(&merged);
        assert_eq!(merged_labels, vec![0, 1, 2, 10, 11]);

        let trimmed = left.delete(1)?;
        let trimmed_labels = int64_labels(&trimmed);
        assert_eq!(trimmed_labels, vec![0, 2]);

        assert!(matches!(
            left.delete(99).unwrap_err(),
            super::IndexError::OutOfBounds {
                position: 99,
                length: 3
            }
        ));
        Ok(())
    }

    #[test]
    fn datetime_index_take_repeat_isin_match_pandas_bbgg3() -> Result<(), super::IndexError> {
        const NS: i64 = 1_000_000_000;
        let a = 1_704_067_200_i64 * NS;
        let b = 1_705_276_800_i64 * NS;
        let c = 1_706_140_800_i64 * NS;
        let dt = super::DatetimeIndex::new(vec![a, b, c]).set_name("ts");

        let taken = dt.take(&[2, 0, 0])?;
        assert_eq!(taken.values(), vec![Some(c), Some(a), Some(a)]);
        assert_eq!(taken.name(), Some("ts"));

        let oob = dt.take(&[3]).unwrap_err();
        assert!(matches!(
            oob,
            super::IndexError::OutOfBounds {
                position: 3,
                length: 3
            }
        ));

        let repeated = dt.repeat(2);
        assert_eq!(
            repeated.values(),
            vec![Some(a), Some(a), Some(b), Some(b), Some(c), Some(c)]
        );
        assert_eq!(repeated.name(), Some("ts"));

        let mask = dt.isin(&[a, c]);
        assert_eq!(mask, vec![true, false, true]);

        let nat_idx = super::DatetimeIndex::new(vec![i64::MIN, a]);
        assert_eq!(nat_idx.isin(&[i64::MIN]), vec![true, false]);
        Ok(())
    }

    #[test]
    fn timedelta_index_take_repeat_isin_match_pandas_bbgg3() -> Result<(), super::IndexError> {
        let td = super::TimedeltaIndex::new(vec![100_i64, 200, 300]).set_name("d");
        let taken = td.take(&[2, 0])?;
        assert_eq!(taken.values(), vec![Some(300), Some(100)]);
        assert_eq!(taken.name(), Some("d"));

        assert!(matches!(
            td.take(&[7]).unwrap_err(),
            super::IndexError::OutOfBounds {
                position: 7,
                length: 3
            }
        ));

        let repeated = td.repeat(2);
        assert_eq!(
            repeated.values(),
            vec![
                Some(100),
                Some(100),
                Some(200),
                Some(200),
                Some(300),
                Some(300)
            ]
        );

        let mask = td.isin(&[200, 999]);
        assert_eq!(mask, vec![false, true, false]);
        Ok(())
    }

    #[test]
    fn period_index_take_repeat_isin_match_pandas_bbgg3() -> Result<(), super::IndexError> {
        use fp_types::{Period, PeriodFreq};
        let p1 = Period::new(10, PeriodFreq::Monthly);
        let p2 = Period::new(11, PeriodFreq::Monthly);
        let p3 = Period::new(12, PeriodFreq::Monthly);
        let pi = super::PeriodIndex::new(vec![p1, p2, p3]).set_name("pp");

        let taken = pi.take(&[2, 1])?;
        assert_eq!(taken.values(), &[p3, p2]);
        assert_eq!(taken.name(), Some("pp"));

        assert!(matches!(
            pi.take(&[5]).unwrap_err(),
            super::IndexError::OutOfBounds {
                position: 5,
                length: 3
            }
        ));

        let repeated = pi.repeat(2);
        assert_eq!(repeated.values(), &[p1, p1, p2, p2, p3, p3]);

        let mask = pi.isin(&[p1, p3]);
        assert_eq!(mask, vec![true, false, true]);
        Ok(())
    }

    #[test]
    fn range_index_take_repeat_isin_match_pandas_bbgg3() -> Result<(), super::IndexError> {
        let r = super::RangeIndex::new(0, 5, 1).unwrap();
        let taken = r.take(&[2, 4, 0])?;
        let labels = int64_labels(&taken);
        assert_eq!(labels, vec![2, 4, 0]);

        assert!(matches!(
            r.take(&[10]).unwrap_err(),
            super::IndexError::OutOfBounds {
                position: 10,
                length: 5
            }
        ));

        let repeated = r.repeat(2);
        let repeat_labels = int64_labels(&repeated);
        assert_eq!(repeat_labels, vec![0, 0, 1, 1, 2, 2, 3, 3, 4, 4]);

        let mask = r.isin(&[1, 3, 99]);
        assert_eq!(mask, vec![false, true, false, true, false]);
        Ok(())
    }

    #[test]
    fn period_index_forwarder_methods_match_pandas_zke9k() {
        use fp_types::{Period, PeriodFreq};
        let p1 = Period::new(10, PeriodFreq::Monthly);
        let p2 = Period::new(11, PeriodFreq::Monthly);
        let p3 = Period::new(12, PeriodFreq::Monthly);
        let pi = super::PeriodIndex::new(vec![p1, p2, p1, p3, p2, p1]).set_name("p");

        let unique = pi.unique();
        assert_eq!(unique.values(), &[p1, p2, p3]);
        assert_eq!(unique.name(), Some("p"));

        let dup_first = pi.duplicated(super::DuplicateKeep::First);
        assert_eq!(dup_first, vec![false, false, true, false, true, true]);

        let dup_last = pi.duplicated(super::DuplicateKeep::Last);
        assert_eq!(dup_last, vec![true, true, true, false, false, false]);

        let dup_none = pi.duplicated(super::DuplicateKeep::None);
        // None marks every position whose value occurs >1 time.
        assert_eq!(dup_none, vec![true, true, true, false, true, true]);

        let dropped = pi.drop_duplicates();
        assert_eq!(dropped.values(), &[p1, p2, p3]);

        let counts = pi.value_counts();
        let total: usize = counts.iter().map(|(_, n)| n).sum();
        assert_eq!(total, pi.len());
        // First entry is the most frequent (p1 with 3 occurrences).
        assert_eq!(counts[0].1, 3);
        let p1_count = counts
            .iter()
            .find_map(|(period, n)| (*period == p1).then_some(*n))
            .expect("p1 should be counted");
        assert_eq!(p1_count, 3);

        let (codes, factor_uniques) = pi.factorize();
        assert_eq!(codes, vec![0, 1, 0, 2, 1, 0]);
        assert_eq!(factor_uniques.values(), &[p1, p2, p3]);
    }

    #[test]
    fn period_index_unique_handles_empty_zke9k() {
        let pi = super::PeriodIndex::new(Vec::new());
        assert!(pi.unique().is_empty());
        assert!(pi.drop_duplicates().is_empty());
        assert!(pi.value_counts().is_empty());
        let (codes, uniques) = pi.factorize();
        assert!(codes.is_empty());
        assert!(uniques.is_empty());
    }

    #[test]
    fn categorical_index_missingness_methods_are_closed_form_c0knj() {
        let cat = super::CategoricalIndex::from_values(
            vec!["a".to_owned(), "b".to_owned(), "c".to_owned()],
            false,
        );
        assert_eq!(cat.isnull(), vec![false, false, false]);
        assert_eq!(cat.notnull(), vec![true, true, true]);
        assert!(!cat.hasnans());
        let dropped = cat.dropna();
        assert_eq!(dropped.labels(), cat.labels());
        let filled = cat.fillna("z");
        assert_eq!(filled.labels(), cat.labels());

        let empty = super::CategoricalIndex::from_values(Vec::<String>::new(), false);
        assert_eq!(empty.isnull(), Vec::<bool>::new());
        assert!(!empty.hasnans());
    }

    #[test]
    fn categorical_index_append_delete_insert_repeat_match_pandas_tns52()
    -> Result<(), super::IndexError> {
        let cat = super::CategoricalIndex::with_categories(
            vec!["a".to_owned(), "b".to_owned(), "c".to_owned()],
            vec![
                "a".to_owned(),
                "b".to_owned(),
                "c".to_owned(),
                "d".to_owned(),
            ],
            false,
        )?
        .set_name("level");

        // append: merge categories.
        let other = super::CategoricalIndex::with_categories(
            vec!["d".to_owned()],
            vec!["d".to_owned(), "e".to_owned()],
            false,
        )?
        .set_name("level");
        let merged = cat.append(&other);
        assert_eq!(
            merged.labels(),
            vec![
                "a".to_owned(),
                "b".to_owned(),
                "c".to_owned(),
                "d".to_owned()
            ]
            .as_slice()
        );
        assert_eq!(merged.name(), Some("level"));
        assert!(merged.categories().contains(&"e".to_owned()));

        // delete OOB.
        assert!(matches!(
            cat.delete(99).unwrap_err(),
            super::IndexError::OutOfBounds {
                position: 99,
                length: 3
            }
        ));
        let trimmed = cat.delete(0)?;
        assert_eq!(
            trimmed.labels(),
            vec!["b".to_owned(), "c".to_owned()].as_slice()
        );

        // insert.
        let inserted = cat.insert(1, "d")?;
        assert_eq!(
            inserted.labels(),
            vec![
                "a".to_owned(),
                "d".to_owned(),
                "b".to_owned(),
                "c".to_owned()
            ]
            .as_slice()
        );
        assert!(cat.insert(1, "zzz").is_err());

        // repeat.
        let repeated = cat.repeat(2);
        assert_eq!(repeated.labels().len(), 6);
        assert_eq!(repeated.labels()[0], "a");
        assert_eq!(repeated.labels()[1], "a");
        assert_eq!(repeated.labels()[2], "b");
        Ok(())
    }

    #[test]
    fn categorical_index_slice_locs_indexer_match_pandas_y93vb() -> Result<(), super::IndexError> {
        let cat = super::CategoricalIndex::with_categories(
            vec![
                "a".to_owned(),
                "b".to_owned(),
                "c".to_owned(),
                "d".to_owned(),
            ],
            vec![
                "a".to_owned(),
                "b".to_owned(),
                "c".to_owned(),
                "d".to_owned(),
            ],
            true,
        )?;
        assert_eq!(cat.slice_locs("b", "c")?, (1, 3));
        assert_eq!(cat.slice_indexer("b", "c")?, 1..3);
        assert_eq!(cat.slice_locs("a", "d")?, (0, 4));

        // Non-monotonic rejects.
        let unsorted = super::CategoricalIndex::from_values(
            vec!["c".to_owned(), "a".to_owned(), "b".to_owned()],
            false,
        );
        assert!(unsorted.slice_locs("a", "c").is_err());
        Ok(())
    }

    #[test]
    fn categorical_index_searchsorted_set_ops_match_pandas_cmvs7() -> Result<(), super::IndexError>
    {
        let cat = super::CategoricalIndex::with_categories(
            vec!["a".to_owned(), "b".to_owned(), "c".to_owned()],
            vec!["a".to_owned(), "b".to_owned(), "c".to_owned()],
            true,
        )?;

        // searchsorted on the sorted utf8 index.
        assert_eq!(cat.searchsorted("b", "left")?, 1);
        assert_eq!(cat.searchsorted("c", "right")?, 3);
        assert!(cat.searchsorted("b", "middle").is_err());

        let other = super::CategoricalIndex::from_values(
            vec!["b".to_owned(), "c".to_owned(), "d".to_owned()],
            false,
        );
        assert_eq!(
            cat.intersection(&other).labels(),
            vec!["b".to_owned(), "c".to_owned()].as_slice()
        );
        assert_eq!(
            cat.union(&other).labels(),
            vec![
                "a".to_owned(),
                "b".to_owned(),
                "c".to_owned(),
                "d".to_owned(),
            ]
            .as_slice()
        );
        assert_eq!(
            cat.difference(&other).labels(),
            vec!["a".to_owned()].as_slice()
        );
        // symmetric_difference: a (only in cat) + d (only in other).
        assert_eq!(
            cat.symmetric_difference(&other).labels(),
            vec!["a".to_owned(), "d".to_owned()].as_slice()
        );
        Ok(())
    }

    #[test]
    fn categorical_index_argmax_argmin_match_pandas_d46wi() -> Result<(), super::IndexError> {
        let cat = super::CategoricalIndex::with_categories(
            vec!["b".to_owned(), "a".to_owned(), "c".to_owned()],
            vec!["a".to_owned(), "b".to_owned(), "c".to_owned()],
            true,
        )?;
        assert_eq!(cat.argmax()?, 2);
        assert_eq!(cat.argmin()?, 1);

        let empty = super::CategoricalIndex::from_values(Vec::<String>::new(), false);
        assert!(empty.argmax().is_err());
        assert!(empty.argmin().is_err());
        Ok(())
    }

    #[test]
    fn categorical_index_forwarders_match_pandas_e2p82() -> Result<(), super::IndexError> {
        let cat = super::CategoricalIndex::with_categories(
            vec![
                "b".to_owned(),
                "a".to_owned(),
                "c".to_owned(),
                "a".to_owned(),
            ],
            vec!["a".to_owned(), "b".to_owned(), "c".to_owned()],
            true,
        )?;

        // argsort: positions sorted by lexicographic label.
        let positions = cat.argsort();
        let labels: Vec<&str> = positions
            .iter()
            .map(|&p| cat.labels()[p].as_str())
            .collect();
        for w in labels.windows(2) {
            assert!(w[0] <= w[1]);
        }

        // take swaps positions.
        let taken = cat.take(&[2, 0, 0])?;
        assert_eq!(
            taken.labels(),
            vec!["c".to_owned(), "b".to_owned(), "b".to_owned()].as_slice()
        );
        assert!(matches!(
            cat.take(&[7]).unwrap_err(),
            super::IndexError::OutOfBounds {
                position: 7,
                length: 4
            }
        ));

        // isin membership.
        assert_eq!(
            cat.isin(&["a".to_owned(), "z".to_owned()]),
            vec![false, true, false, true]
        );

        // get_loc finds first; missing rejects.
        assert_eq!(cat.get_loc("c")?, 2);
        assert!(cat.get_loc("zzz").is_err());

        // min/max with ordered=true uses category order.
        assert_eq!(cat.min(), Some("a"));
        assert_eq!(cat.max(), Some("c"));

        // Empty.
        let empty = super::CategoricalIndex::from_values(Vec::<String>::new(), false);
        assert_eq!(empty.min(), None);
        assert_eq!(empty.max(), None);
        assert!(empty.argsort().is_empty());
        Ok(())
    }

    #[test]
    fn categorical_index_category_management_match_pandas_zy2vd() -> Result<(), super::IndexError> {
        let cat = super::CategoricalIndex::with_categories(
            vec!["a".to_owned(), "b".to_owned()],
            vec!["a".to_owned(), "b".to_owned(), "c".to_owned()],
            false,
        )?;

        // as_ordered / as_unordered.
        assert!(cat.as_ordered().ordered());
        assert!(!cat.as_ordered().as_unordered().ordered());

        // add_categories: appends "d".
        let added = cat.add_categories(vec!["d".to_owned()])?;
        assert_eq!(
            added.categories(),
            vec![
                "a".to_owned(),
                "b".to_owned(),
                "c".to_owned(),
                "d".to_owned()
            ]
            .as_slice()
        );
        // Adding a duplicate rejects.
        assert!(cat.add_categories(vec!["a".to_owned()]).is_err());

        // remove_categories drops "c" (unused).
        let pruned = cat.remove_categories(&["c".to_owned()])?;
        assert_eq!(
            pruned.categories(),
            vec!["a".to_owned(), "b".to_owned()].as_slice()
        );
        // Trying to remove a category that's still in use rejects.
        assert!(cat.remove_categories(&["a".to_owned()]).is_err());
        // Removing a missing category rejects.
        assert!(cat.remove_categories(&["zzz".to_owned()]).is_err());

        // remove_unused_categories trims "c" automatically.
        let trimmed = cat.remove_unused_categories();
        assert_eq!(
            trimmed.categories(),
            vec!["a".to_owned(), "b".to_owned()].as_slice()
        );

        // set_categories: extending to {a, b, c, d}.
        let extended = cat.set_categories(vec![
            "a".to_owned(),
            "b".to_owned(),
            "c".to_owned(),
            "d".to_owned(),
        ])?;
        assert_eq!(extended.categories().len(), 4);
        // set_categories must include every current label.
        assert!(
            cat.set_categories(vec!["b".to_owned(), "c".to_owned()])
                .is_err()
        );

        // rename_categories: a→A, b→B, c→C.
        let renamed =
            cat.rename_categories(vec!["A".to_owned(), "B".to_owned(), "C".to_owned()])?;
        assert_eq!(
            renamed.labels(),
            vec!["A".to_owned(), "B".to_owned()].as_slice()
        );
        assert_eq!(
            renamed.categories(),
            vec!["A".to_owned(), "B".to_owned(), "C".to_owned()].as_slice()
        );
        // Wrong length rejects.
        assert!(cat.rename_categories(vec!["X".to_owned()]).is_err());

        // reorder_categories: permutation passes; ordered flag flips.
        let reordered =
            cat.reorder_categories(vec!["c".to_owned(), "b".to_owned(), "a".to_owned()], true)?;
        assert!(reordered.ordered());
        assert_eq!(
            reordered.categories(),
            vec!["c".to_owned(), "b".to_owned(), "a".to_owned()].as_slice()
        );
        // Non-permutation rejects.
        assert!(
            cat.reorder_categories(vec!["a".to_owned(), "b".to_owned(), "x".to_owned()], false)
                .is_err()
        );
        // Duplicate-bearing input rejects.
        assert!(
            cat.reorder_categories(vec!["a".to_owned(), "a".to_owned(), "b".to_owned()], false)
                .is_err()
        );

        Ok(())
    }

    #[test]
    fn categorical_index_forwarder_methods_match_pandas_i1q1c() {
        let labels = vec![
            "low".to_owned(),
            "high".to_owned(),
            "low".to_owned(),
            "med".to_owned(),
            "high".to_owned(),
            "low".to_owned(),
        ];
        let categorical =
            super::CategoricalIndex::from_values(labels.clone(), false).set_name("level");

        // unique: first-seen low, high, med.
        let unique = categorical.unique();
        assert_eq!(
            unique.labels(),
            vec!["low".to_owned(), "high".to_owned(), "med".to_owned()].as_slice()
        );
        assert_eq!(unique.name(), Some("level"));

        // duplicated keep=First: positions 2, 4, 5 are duplicates of earlier.
        let dup_first = categorical.duplicated(super::DuplicateKeep::First);
        assert_eq!(dup_first, vec![false, false, true, false, true, true]);

        // drop_duplicates returns same as unique.
        let dropped = categorical.drop_duplicates();
        assert_eq!(dropped.labels(), unique.labels());

        // value_counts: low=3, high=2, med=1; total = 6 = len.
        let counts = categorical.value_counts();
        let total: usize = counts.iter().map(|(_, n)| n).sum();
        assert_eq!(total, categorical.len());
        let low_count = counts
            .iter()
            .find_map(|(label, n)| (label == "low").then_some(*n))
            .expect("low should be counted");
        assert_eq!(low_count, 3);
        // First entry is the most frequent (descending sort).
        assert_eq!(counts[0].1, 3);

        // factorize: codes encode first-seen positions; uniques == unique().
        let (codes, factor_uniques) = categorical.factorize();
        assert_eq!(codes.len(), categorical.len());
        assert_eq!(codes, vec![0, 1, 0, 2, 1, 0]);
        assert_eq!(factor_uniques.labels(), unique.labels());
    }

    #[test]
    fn categorical_index_unique_preserves_categories_and_ordered_i1q1c() {
        let labels = vec!["a".to_owned(), "b".to_owned(), "a".to_owned()];
        let categories = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
        let cat = super::CategoricalIndex::with_categories(labels, categories.clone(), true)
            .expect("with_categories");
        let unique = cat.unique();
        assert_eq!(unique.categories(), categories.as_slice());
        assert!(unique.ordered());
    }

    #[test]
    fn timedelta_index_forwarder_methods_match_index_vq4pf() -> Result<(), super::IndexError> {
        let a: i64 = 1_000;
        let b: i64 = 2_000;
        let c: i64 = 3_000;
        let nat = fp_types::Timedelta::NAT;

        // a, c, b, a, NAT, c (duplicates + NAT)
        let td = super::TimedeltaIndex::new(vec![a, c, b, a, nat, c]);

        assert_eq!(td.argmax()?, 1);
        assert_eq!(td.argmin()?, 0);

        let positions = td.argsort();
        assert_eq!(positions.len(), td.len());

        let unique = td.unique()?;
        assert_eq!(unique.values(), vec![Some(a), Some(c), Some(b), None]);

        let (codes, uniques) = td.factorize()?;
        assert_eq!(codes.len(), td.len());
        assert_eq!(uniques.values(), vec![Some(a), Some(c), Some(b)]);
        assert_eq!(codes[4], -1);

        let counts = td.value_counts();
        let total: usize = counts.iter().map(|(_, n)| n).sum();
        assert_eq!(total, 5); // NAT dropped

        let dup_first = td.duplicated(super::DuplicateKeep::First);
        assert_eq!(dup_first, vec![false, false, false, true, false, true]);

        let deduped = td.drop_duplicates()?;
        assert_eq!(deduped.values(), vec![Some(a), Some(c), Some(b), None]);

        let dropped = td.dropna();
        assert_eq!(
            dropped.values(),
            vec![Some(a), Some(c), Some(b), Some(a), Some(c)]
        );
        Ok(())
    }

    #[test]
    fn timedelta_index_argmax_argmin_reject_empty_vq4pf() {
        let empty = super::TimedeltaIndex::new(vec![]);
        let err_max = empty.argmax().unwrap_err();
        assert!(matches!(
            err_max,
            super::IndexError::InvalidArgument(ref message)
                if message == "attempt to get argmax of an empty sequence"
        ));
        let err_min = empty.argmin().unwrap_err();
        assert!(matches!(
            err_min,
            super::IndexError::InvalidArgument(ref message)
                if message == "attempt to get argmin of an empty sequence"
        ));

        let only_nat =
            super::TimedeltaIndex::new(vec![fp_types::Timedelta::NAT, fp_types::Timedelta::NAT]);
        assert!(only_nat.argmax().is_err());
        assert!(only_nat.argmin().is_err());
    }

    #[test]
    fn timedelta_index_dropna_preserves_name_vq4pf() {
        let td =
            super::TimedeltaIndex::new(vec![fp_types::Timedelta::NAT, 0_i64]).set_name("delta");
        let dropped = td.dropna();
        assert_eq!(dropped.values(), vec![Some(0)]);
        assert_eq!(dropped.name(), Some("delta"));
    }

    #[test]
    fn datetime_index_forwarder_methods_match_index_z9guv() -> Result<(), super::IndexError> {
        const NS: i64 = 1_000_000_000;
        let a = 1_704_067_200_i64 * NS;
        let b = 1_705_276_800_i64 * NS;
        let c = 1_706_140_800_i64 * NS;

        // a, c, b, a, NAT, c (duplicates + NAT to exercise every branch).
        let dt = super::DatetimeIndex::new(vec![a, c, b, a, i64::MIN, c]);

        // argmax / argmin skip NAT to match pandas skipna=True default.
        assert_eq!(dt.argmax()?, 1); // c at position 1 is first-seen max
        assert_eq!(dt.argmin()?, 0); // a at position 0 is first-seen min

        // argsort returns positions in ascending label order (NAT sorts lowest
        // because i64::MIN < every datetime). Stable on ties.
        let positions = dt.argsort();
        assert_eq!(positions.len(), dt.len());
        let sorted_labels: Vec<&super::IndexLabel> = positions
            .iter()
            .map(|&p| &dt.as_index().labels()[p])
            .collect();
        for w in sorted_labels.windows(2) {
            assert!(w[0].cmp(w[1]).is_le());
        }

        let unique = dt.unique()?;
        // First-seen order including NAT: a, c, b, NAT.
        assert_eq!(unique.values(), vec![Some(a), Some(c), Some(b), None]);

        let (codes, uniques) = dt.factorize()?;
        assert_eq!(codes.len(), dt.len());
        // factorize skips NAT in the uniques and emits -1 codes for NAT inputs
        // (matches pandas).
        assert_eq!(uniques.values(), vec![Some(a), Some(c), Some(b)]);
        // Position 4 is the NAT input.
        assert_eq!(codes[4], -1);

        let counts = dt.value_counts();
        // value_counts drops NAT by default (matches pandas dropna=True), so
        // total = 5: a:2, c:2, b:1.
        let total_count: usize = counts.iter().map(|(_, n)| n).sum();
        assert_eq!(total_count, 5);
        let a_count = counts
            .iter()
            .find_map(|(label, n)| match label {
                super::IndexLabel::Datetime64(nanos) if *nanos == a => Some(*n),
                _ => None,
            })
            .expect("a should be counted");
        assert_eq!(a_count, 2);

        let dup_first = dt.duplicated(super::DuplicateKeep::First);
        // Positions 3 (second a) and 5 (second c) are duplicates.
        assert_eq!(dup_first, vec![false, false, false, true, false, true]);

        let deduped = dt.drop_duplicates()?;
        // drop_duplicates preserves the NAT entry — only literal duplicates go.
        assert_eq!(deduped.values(), vec![Some(a), Some(c), Some(b), None]);

        let dropped = dt.dropna();
        assert_eq!(
            dropped.values(),
            vec![Some(a), Some(c), Some(b), Some(a), Some(c)]
        );
        Ok(())
    }

    #[test]
    fn datetime_index_argmax_argmin_reject_empty_z9guv() {
        let empty = super::DatetimeIndex::new(vec![]);
        let err_max = empty.argmax().unwrap_err();
        assert!(matches!(
            err_max,
            super::IndexError::InvalidArgument(ref message)
                if message == "attempt to get argmax of an empty sequence"
        ));
        let err_min = empty.argmin().unwrap_err();
        assert!(matches!(
            err_min,
            super::IndexError::InvalidArgument(ref message)
                if message == "attempt to get argmin of an empty sequence"
        ));
        assert!(empty.argsort().is_empty());
        assert!(empty.dropna().is_empty());
    }

    #[test]
    fn datetime_index_dropna_preserves_name_z9guv() {
        let dt = super::DatetimeIndex::new(vec![i64::MIN, 0_i64, i64::MIN]).set_name("ts");
        let dropped = dt.dropna();
        assert_eq!(dropped.values(), vec![Some(0)]);
        assert_eq!(dropped.name(), Some("ts"));
    }

    #[test]
    fn datetime_index_asi8_round_trips_nanos_teeck() {
        const NS: i64 = 1_000_000_000;
        let total: i64 = 1_704_067_200_i64 * NS + 123;
        let dt = super::DatetimeIndex::new(vec![total, i64::MIN, 0]);
        assert_eq!(dt.asi8(), vec![total, i64::MIN, 0]);

        let empty = super::DatetimeIndex::new(vec![]);
        assert!(empty.asi8().is_empty());
    }

    #[test]
    fn datetime_index_strftime_formats_each_label_teeck() {
        const NS: i64 = 1_000_000_000;
        // 2024-01-15T12:34:56.789Z:
        //   2024-01-01 00:00:00Z = 1704067200 sec.
        //   + 14 * 86400 = 1209600  -> 2024-01-15 00:00:00 = 1705276800
        //   + 12*3600 + 34*60 + 56  -> 1705322096
        //   * 1e9 + 789_000_000 ns
        let with_ms: i64 = 1_705_322_096_i64 * NS + 789_000_000;
        let dt = super::DatetimeIndex::new(vec![with_ms, i64::MIN]);
        let formatted = dt.strftime("%Y-%m-%dT%H:%M:%S%.3f");
        assert_eq!(
            formatted,
            vec![Some("2024-01-15T12:34:56.789".to_owned()), None]
        );
    }

    #[test]
    fn format_datetime_ns_keeps_subsecond_and_pre_epoch_precision_dt64fmt() {
        const NS: i64 = 1_000_000_000;
        let base: i64 = 1_705_322_096_i64 * NS; // 2024-01-15 12:34:56 UTC

        // No subsecond: unchanged plain "%Y-%m-%d %H:%M:%S".
        assert_eq!(super::format_datetime_ns(base), "2024-01-15 12:34:56");
        // Millisecond fraction is carried (was DROPPED before dt64fmt), with the
        // same trailing-zero trimming format_naive_datetime uses.
        assert_eq!(
            super::format_datetime_ns(base + 789_000_000),
            "2024-01-15 12:34:56.789"
        );
        // Full nanosecond precision survives.
        assert_eq!(
            super::format_datetime_ns(base + 123_456_789),
            "2024-01-15 12:34:56.123456789"
        );
        // Pre-epoch instants decompose with floor semantics: -0.5s is
        // 1969-12-31 23:59:59.5, not 1970-01-01 00:00:00.5.
        assert_eq!(
            super::format_datetime_ns(-500_000_000),
            "1969-12-31 23:59:59.5"
        );
        // NaT sentinel is preserved.
        assert_eq!(super::format_datetime_ns(i64::MIN), "NaT");
    }

    #[test]
    fn timedelta_index_asi8_microseconds_nanoseconds_match_pandas_teeck() -> Result<(), &'static str>
    {
        // 1 day + 2:34:56.789012345
        let one_day = fp_types::Timedelta::NANOS_PER_DAY;
        let extra = 2 * fp_types::Timedelta::NANOS_PER_HOUR
            + 34 * fp_types::Timedelta::NANOS_PER_MIN
            + 56 * fp_types::Timedelta::NANOS_PER_SEC
            + 789_012_345;
        let total = one_day + extra;
        let td = super::TimedeltaIndex::new(vec![total, fp_types::Timedelta::NAT, 0, -1]);

        assert_eq!(td.asi8(), vec![total, fp_types::Timedelta::NAT, 0, -1]);
        // microseconds: 789_012_345 % 1_000_000_000 / 1_000 == 789_012
        assert_eq!(
            td.microseconds(),
            vec![Some(789_012), None, Some(0), Some(999_999)]
        );
        // nanoseconds: 789_012_345 % 1_000 == 345
        assert_eq!(td.nanoseconds(), vec![Some(345), None, Some(0), Some(999)]);

        let components = td.components();
        let positive = components
            .first()
            .copied()
            .flatten()
            .ok_or("positive components")?;
        assert_eq!(positive.days, 1);
        assert_eq!(positive.hours, 2);
        assert_eq!(positive.minutes, 34);
        assert_eq!(positive.seconds, 56);
        assert_eq!(positive.milliseconds, 789);
        assert_eq!(positive.microseconds, 12);
        assert_eq!(positive.nanoseconds, 345);

        assert_eq!(
            components.get(1).copied().flatten().map(|row| row.days),
            None
        );

        let zero = components
            .get(2)
            .copied()
            .flatten()
            .ok_or("zero components")?;
        assert_eq!(zero.days, 0);
        assert_eq!(zero.hours, 0);
        assert_eq!(zero.minutes, 0);
        assert_eq!(zero.seconds, 0);
        assert_eq!(zero.milliseconds, 0);
        assert_eq!(zero.microseconds, 0);
        assert_eq!(zero.nanoseconds, 0);

        let negative = components
            .get(3)
            .copied()
            .flatten()
            .ok_or("negative components")?;
        assert_eq!(negative.days, -1);
        assert_eq!(negative.hours, 23);
        assert_eq!(negative.minutes, 59);
        assert_eq!(negative.seconds, 59);
        assert_eq!(negative.milliseconds, 999);
        assert_eq!(negative.microseconds, 999);
        assert_eq!(negative.nanoseconds, 999);

        Ok(())
    }

    #[test]
    fn datetime_index_month_name_and_day_name_match_pandas_fqkiu() {
        // 2024-01-15 (Monday in January), 2024-12-31 (Tuesday in December),
        // i64::MIN (NAT).
        const NS: i64 = 1_000_000_000;
        let mon_jan: i64 = 1_705_276_800_i64 * NS;
        let tue_dec: i64 = 1_735_603_200_i64 * NS;
        let dt = super::DatetimeIndex::new(vec![mon_jan, tue_dec, i64::MIN]);

        assert_eq!(
            dt.month_name(),
            vec![
                Some("January".to_owned()),
                Some("December".to_owned()),
                None
            ]
        );
        assert_eq!(
            dt.day_name(),
            vec![Some("Monday".to_owned()), Some("Tuesday".to_owned()), None]
        );
    }

    #[test]
    fn datetime_index_normalize_truncates_to_midnight_utc_fqkiu() {
        const NS: i64 = 1_000_000_000;
        // 2024-01-15 12:34:56.789Z plus midnight 2024-01-21Z plus NAT.
        let mid_day: i64 = 1_705_276_800_i64 * NS + 12 * 3600 * NS + 34 * 60 * NS + 56 * NS + 789;
        let midnight: i64 = 1_705_795_200_i64 * NS;
        let nat = i64::MIN;

        let dt = super::DatetimeIndex::new(vec![mid_day, midnight, nat]).set_name("when");
        let normed = dt.normalize();

        // Each non-NAT entry is now at midnight; NAT stays NAT; name preserved.
        assert_eq!(
            normed.values(),
            vec![Some(1_705_276_800_i64 * NS), Some(midnight), None]
        );
        assert_eq!(normed.name(), Some("when"));
        assert!(normed.is_normalized());
    }

    #[test]
    fn datetime_index_is_normalized_returns_false_when_any_non_midnight_fqkiu() {
        const NS: i64 = 1_000_000_000;
        let mid_day: i64 = 1_705_276_800_i64 * NS + 1; // 1ns past midnight
        let midnight: i64 = 1_705_795_200_i64 * NS;
        let mixed = super::DatetimeIndex::new(vec![midnight, mid_day]);
        assert!(!mixed.is_normalized());

        let only_midnight = super::DatetimeIndex::new(vec![midnight]);
        assert!(only_midnight.is_normalized());

        let only_nat = super::DatetimeIndex::new(vec![i64::MIN, i64::MIN]);
        assert!(only_nat.is_normalized());

        let empty = super::DatetimeIndex::new(vec![]);
        assert!(empty.is_normalized());
    }

    #[test]
    fn datetime_index_feb_28_in_non_leap_year_is_month_end_qy7yd() {
        // 2023-02-28 00:00:00Z is the month-end of February 2023 (28 days).
        let feb_28_2023: i64 = 1_677_542_400_i64 * 1_000_000_000;
        let dt = super::DatetimeIndex::new(vec![feb_28_2023]);
        assert_eq!(dt.is_month_end(), vec![Some(true)]);
    }

    #[test]
    fn datetime_index_leap_year_century_rule_k860x() {
        // 2000-06-15 (leap), 2100-06-15 (not leap), 2024-02-15 (leap),
        // 2023-02-15 (not leap), Feb in leap vs non-leap year.
        let y2000: i64 = 960_076_800 * 1_000_000_000;
        let y2100: i64 = 4_117_046_400 * 1_000_000_000;
        let y2024feb: i64 = 1_708_002_000 * 1_000_000_000;
        let y2023feb: i64 = 1_676_466_000 * 1_000_000_000;

        let dt = super::DatetimeIndex::new(vec![y2000, y2100, y2024feb, y2023feb]);
        assert_eq!(
            dt.is_leap_year(),
            vec![Some(true), Some(false), Some(true), Some(false)]
        );
        // Feb in leap year -> 29 days; non-leap -> 28.
        let dim = dt.days_in_month();
        assert_eq!(dim[2], Some(29));
        assert_eq!(dim[3], Some(28));
    }

    #[test]
    fn datetime_index_time_of_day_accessors_handle_empty_znejf() {
        let dt = super::DatetimeIndex::new(vec![]);
        assert!(dt.hour().is_empty());
        assert!(dt.minute().is_empty());
        assert!(dt.second().is_empty());
        assert!(dt.microsecond().is_empty());
        assert!(dt.nanosecond().is_empty());
    }

    #[test]
    fn range_index_missingness_methods_are_closed_form_a4fih() {
        let asc = super::RangeIndex::new(0, 5, 1).unwrap();
        assert_eq!(asc.isna(), vec![false; 5]);
        assert_eq!(asc.isnull(), vec![false; 5]);
        assert_eq!(asc.notna(), vec![true; 5]);
        assert_eq!(asc.notnull(), vec![true; 5]);
        assert!(!asc.hasnans());
        assert!(asc.dropna().equals(&asc));
        assert!(asc.fillna(99).equals(&asc));

        let desc = super::RangeIndex::new(10, 0, -2).unwrap();
        assert_eq!(desc.isna().len(), desc.len());
        assert!(!desc.hasnans());
        assert!(desc.dropna().equals(&desc));

        let empty = super::RangeIndex::new(0, 0, 1).unwrap();
        assert_eq!(empty.isna(), Vec::<bool>::new());
        assert_eq!(empty.notna(), Vec::<bool>::new());
        assert!(!empty.hasnans());
        assert!(empty.dropna().is_empty());
        assert!(empty.fillna(0).is_empty());
    }

    #[test]
    fn range_index_format_stringifies_each_value_a4fih() {
        let asc = super::RangeIndex::new(0, 4, 1).unwrap();
        assert_eq!(asc.format(), vec!["0", "1", "2", "3"]);

        let desc = super::RangeIndex::new(5, 0, -2).unwrap();
        assert_eq!(desc.format(), vec!["5", "3", "1"]);

        let empty = super::RangeIndex::new(0, 0, 1).unwrap();
        assert_eq!(empty.format(), Vec::<String>::new());
    }

    #[test]
    fn range_index_factorize_is_identity_a4fih() {
        let asc = super::RangeIndex::new(0, 5, 1).unwrap();
        let (codes, uniques) = asc.factorize();
        assert_eq!(codes, vec![0, 1, 2, 3, 4]);
        assert!(uniques.equals(&asc));

        let desc = super::RangeIndex::new(10, 0, -2).unwrap();
        let (desc_codes, desc_uniques) = desc.factorize();
        assert_eq!(desc_codes, (0..desc.len()).collect::<Vec<_>>());
        assert!(desc_uniques.equals(&desc));

        let empty = super::RangeIndex::new(0, 0, 1).unwrap();
        let (empty_codes, empty_uniques) = empty.factorize();
        assert!(empty_codes.is_empty());
        assert!(empty_uniques.is_empty());
    }

    #[test]
    fn range_index_duplicated_drop_duplicates_are_no_ops_mrchb() {
        let asc = super::RangeIndex::new(0, 5, 1).unwrap();
        for keep in [
            super::DuplicateKeep::First,
            super::DuplicateKeep::Last,
            super::DuplicateKeep::None,
        ] {
            assert_eq!(asc.duplicated(keep), vec![false; asc.len()]);
        }
        let cloned = asc.drop_duplicates();
        assert!(cloned.equals(&asc));
        assert_eq!(cloned.len(), asc.len());

        let empty = super::RangeIndex::new(0, 0, 1).unwrap();
        assert_eq!(
            empty.duplicated(super::DuplicateKeep::First),
            Vec::<bool>::new()
        );
        assert!(empty.drop_duplicates().is_empty());
    }

    #[test]
    fn multi_index_asof_rejects_tuple_comparison_d89fe13() -> Result<(), super::IndexError> {
        let string_level = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
        ])?;
        let int_level = MultiIndex::from_tuples(vec![
            vec![1_i64.into(), "a".into()],
            vec![2_i64.into(), "b".into()],
        ])?;

        let string_err = string_level
            .asof(&[IndexLabel::Utf8("a".into()), IndexLabel::Int64(1)])
            .unwrap_err();
        let int_err = int_level
            .asof(&[IndexLabel::Int64(1), IndexLabel::Utf8("a".into())])
            .unwrap_err();

        assert!(matches!(
            string_err,
            super::IndexError::InvalidArgument(message)
                if message == "'<' not supported between instances of 'tuple' and 'str'"
        ));
        assert!(matches!(
            int_err,
            super::IndexError::InvalidArgument(message)
                if message == "'<' not supported between instances of 'tuple' and 'int'"
        ));
        assert_eq!(MultiIndex::from_tuples(Vec::new())?.asof(&[])?, None);

        Ok(())
    }

    #[test]
    fn multi_index_asof_locs_rejects_mask_and_broadcast_paths_d89fe14()
    -> Result<(), super::IndexError> {
        let source = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["a".into(), 3_i64.into()],
            vec!["b".into(), 2_i64.into()],
        ])?;
        let where_index = MultiIndex::from_tuples(vec![
            vec!["a".into(), 0_i64.into()],
            vec!["a".into(), 2_i64.into()],
            vec!["b".into(), 2_i64.into()],
        ])?;

        let no_mask = source.asof_locs(&where_index, None).unwrap_err();
        let mismatched_mask = source
            .asof_locs(&where_index, Some(&[true, true]))
            .unwrap_err();
        let empty_take = source
            .asof_locs(&where_index, Some(&[false, false, false]))
            .unwrap_err();
        let broadcast = source
            .asof_locs(&where_index, Some(&[true, false, true]))
            .unwrap_err();
        let empty_source = MultiIndex::from_arrays(vec![Vec::new(), Vec::new()])?;
        let empty_mask = empty_source
            .asof_locs(&empty_source, Some(&[]))
            .unwrap_err();

        assert!(matches!(
            no_mask,
            super::IndexError::InvalidArgument(message)
                if message == "object too deep for desired array"
        ));
        assert!(matches!(
            mismatched_mask,
            super::IndexError::InvalidArgument(message)
                if message == "boolean index did not match indexed array along axis 0; size of axis is 3 but size of corresponding boolean axis is 2"
        ));
        assert!(matches!(
            empty_take,
            super::IndexError::InvalidArgument(message)
                if message == "cannot do a non-empty take from an empty axes."
        ));
        assert!(matches!(
            broadcast,
            super::IndexError::InvalidArgument(message)
                if message == "operands could not be broadcast together with shapes (3,) (2,)"
        ));
        assert!(matches!(
            empty_mask,
            super::IndexError::InvalidArgument(message)
                if message == "attempt to get argmax of an empty sequence"
        ));

        Ok(())
    }

    #[test]
    fn multi_index_drop_duplicates_append_repeat_and_dropna() {
        let left = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
            vec![IndexLabel::Datetime64(i64::MIN), 3_i64.into()],
            vec![
                IndexLabel::Datetime64(i64::MIN),
                IndexLabel::Timedelta64(Timedelta::NAT),
            ],
        ])
        .unwrap()
        .set_names(vec![Some("letter".into()), Some("number".into())]);

        assert_eq!(
            left.drop_duplicates().to_list(),
            vec![
                vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(1)],
                vec![IndexLabel::Utf8("b".into()), IndexLabel::Int64(2)],
                vec![IndexLabel::Datetime64(i64::MIN), IndexLabel::Int64(3)],
                vec![
                    IndexLabel::Datetime64(i64::MIN),
                    IndexLabel::Timedelta64(Timedelta::NAT),
                ],
            ]
        );
        assert_eq!(
            left.dropna().to_list(),
            vec![
                vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(1)],
                vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(1)],
                vec![IndexLabel::Utf8("b".into()), IndexLabel::Int64(2)],
            ]
        );
        assert_eq!(left.dropna_all().len(), 4);

        let right = MultiIndex::from_tuples(vec![vec!["c".into(), 3_i64.into()]])
            .unwrap()
            .set_names(vec![Some("letter".into()), Some("other".into())]);
        let appended = left.append(&right).unwrap();
        assert_eq!(appended.len(), 6);
        assert_eq!(appended.names(), &[Some("letter".into()), None]);

        let repeated = right.repeat(2);
        assert_eq!(
            repeated.to_list(),
            vec![
                vec![IndexLabel::Utf8("c".into()), IndexLabel::Int64(3)],
                vec![IndexLabel::Utf8("c".into()), IndexLabel::Int64(3)],
            ]
        );
        assert_eq!(right.repeat(0).len(), 0);
    }

    #[test]
    fn multi_index_insert_delete_and_drop_tuples() {
        let mi = MultiIndex::from_tuples(vec![
            vec!["b".into(), 2_i64.into()],
            vec!["a".into(), 2_i64.into()],
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
        ])
        .unwrap()
        .set_names(vec![Some("letter".into()), Some("number".into())]);

        let inserted = mi.insert(1, vec!["z".into(), 9_i64.into()]).unwrap();
        assert_eq!(
            inserted.to_list(),
            vec![
                vec![IndexLabel::Utf8("b".into()), IndexLabel::Int64(2)],
                vec![IndexLabel::Utf8("z".into()), IndexLabel::Int64(9)],
                vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(2)],
                vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(1)],
                vec![IndexLabel::Utf8("b".into()), IndexLabel::Int64(2)],
            ]
        );
        assert_eq!(inserted.names(), mi.names());

        let deleted = inserted.delete(1).unwrap();
        assert_eq!(deleted, mi);
        assert!(mi.insert(0, vec!["short".into()]).is_err());
        assert!(mi.delete(99).is_err());

        let dropped = mi
            .drop(&[vec![IndexLabel::Utf8("b".into()), IndexLabel::Int64(2)]])
            .unwrap();
        assert_eq!(
            dropped.to_list(),
            vec![
                vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(2)],
                vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(1)],
            ]
        );
        assert!(
            mi.drop(&[vec![
                IndexLabel::Utf8("missing".into()),
                IndexLabel::Int64(0)
            ]])
            .is_err()
        );
    }

    #[test]
    fn multi_index_factorize_sort_and_reduce_tuples() {
        let mi = MultiIndex::from_tuples(vec![
            vec!["b".into(), 2_i64.into()],
            vec!["a".into(), 2_i64.into()],
            vec!["a".into(), 1_i64.into()],
            vec!["b".into(), 2_i64.into()],
            vec!["a".into(), 2_i64.into()],
            vec!["c".into(), 3_i64.into()],
        ])
        .unwrap()
        .set_names(vec![Some("letter".into()), Some("number".into())]);

        let (codes, uniques) = mi.factorize();
        assert_eq!(codes, vec![0, 1, 2, 0, 1, 3]);
        assert_eq!(
            uniques.to_list(),
            vec![
                vec![IndexLabel::Utf8("b".into()), IndexLabel::Int64(2)],
                vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(2)],
                vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(1)],
                vec![IndexLabel::Utf8("c".into()), IndexLabel::Int64(3)],
            ]
        );
        assert_eq!(uniques.names(), mi.names());
        assert_eq!(mi.unique(), uniques);
        assert_eq!(mi.nunique(), 4);
        assert_eq!(
            mi.value_counts(),
            vec![
                (vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(2)], 2),
                (vec![IndexLabel::Utf8("b".into()), IndexLabel::Int64(2)], 2),
                (vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(1)], 1),
                (vec![IndexLabel::Utf8("c".into()), IndexLabel::Int64(3)], 1),
            ]
        );

        let sorted = mi.sort_values();
        assert_eq!(
            sorted.to_list(),
            vec![
                vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(1)],
                vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(2)],
                vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(2)],
                vec![IndexLabel::Utf8("b".into()), IndexLabel::Int64(2)],
                vec![IndexLabel::Utf8("b".into()), IndexLabel::Int64(2)],
                vec![IndexLabel::Utf8("c".into()), IndexLabel::Int64(3)],
            ]
        );
        let (sortlevel, order) = mi.sortlevel();
        assert_eq!(sortlevel, sorted);
        assert_eq!(order, vec![2, 1, 4, 0, 3, 5]);
        assert_eq!(mi.sort(), sorted);
        assert_eq!(
            mi.min().unwrap(),
            vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(1)]
        );
        assert_eq!(
            mi.max().unwrap(),
            vec![IndexLabel::Utf8("c".into()), IndexLabel::Int64(3)]
        );
        assert_eq!(mi.argmin(), Some(2));
        assert_eq!(mi.argmax(), Some(5));

        let empty = MultiIndex::from_tuples(Vec::new()).unwrap();
        assert_eq!(empty.min(), None);
        assert_eq!(empty.max(), None);
        assert_eq!(empty.argmin(), None);
        assert_eq!(empty.argmax(), None);
    }

    #[test]
    fn multi_index_tuple_set_ops_preserve_order_and_shared_names() {
        let left = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into()],
            vec!["a".into(), 2_i64.into()],
            vec!["b".into(), 1_i64.into()],
            vec!["a".into(), 1_i64.into()],
        ])
        .unwrap()
        .set_names(vec![Some("letter".into()), Some("number".into())]);
        let right = MultiIndex::from_tuples(vec![
            vec!["a".into(), 2_i64.into()],
            vec!["c".into(), 3_i64.into()],
        ])
        .unwrap()
        .set_names(vec![Some("letter".into()), Some("other".into())]);

        let intersection = left.intersection(&right).unwrap();
        assert_eq!(
            intersection.to_list(),
            vec![vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(2)]]
        );
        assert_eq!(intersection.names(), &[Some("letter".into()), None]);

        assert_eq!(
            left.union(&right).unwrap().to_list(),
            vec![
                vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(1)],
                vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(2)],
                vec![IndexLabel::Utf8("b".into()), IndexLabel::Int64(1)],
                vec![IndexLabel::Utf8("c".into()), IndexLabel::Int64(3)],
            ]
        );
        assert_eq!(
            left.difference(&right).unwrap().to_list(),
            vec![
                vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(1)],
                vec![IndexLabel::Utf8("b".into()), IndexLabel::Int64(1)],
            ]
        );
        assert_eq!(
            left.symmetric_difference(&right).unwrap().to_list(),
            vec![
                vec![IndexLabel::Utf8("a".into()), IndexLabel::Int64(1)],
                vec![IndexLabel::Utf8("b".into()), IndexLabel::Int64(1)],
                vec![IndexLabel::Utf8("c".into()), IndexLabel::Int64(3)],
            ]
        );
    }

    #[test]
    fn multi_index_reorder_levels() {
        let mi = MultiIndex::from_tuples(vec![
            vec!["a".into(), 1_i64.into(), "x".into()],
            vec!["b".into(), 2_i64.into(), "y".into()],
        ])
        .unwrap()
        .set_names(vec![
            Some("letter".into()),
            Some("number".into()),
            Some("code".into()),
        ]);

        // Reorder: [2, 0, 1] → code, letter, number.
        let reordered = mi.reorder_levels(&[2, 0, 1]).unwrap();
        assert_eq!(reordered.nlevels(), 3);
        assert_eq!(
            reordered.names(),
            &[
                Some("code".into()),
                Some("letter".into()),
                Some("number".into())
            ]
        );

        // First row should be ("x", "a", 1).
        let tuple = reordered.get_tuple(0).unwrap();
        assert_eq!(tuple[0], &IndexLabel::Utf8("x".into()));
        assert_eq!(tuple[1], &IndexLabel::Utf8("a".into()));
        assert_eq!(tuple[2], &IndexLabel::Int64(1));
    }

    #[test]
    fn multi_index_reorder_levels_identity() {
        let mi = MultiIndex::from_tuples(vec![vec!["a".into(), 1_i64.into()]]).unwrap();

        // Identity reorder [0, 1] should be a no-op.
        let same = mi.reorder_levels(&[0, 1]).unwrap();
        assert_eq!(same, mi);
    }

    #[test]
    fn multi_index_reorder_levels_wrong_length_errors() {
        let mi = MultiIndex::from_tuples(vec![vec!["a".into(), 1_i64.into()]]).unwrap();

        assert!(mi.reorder_levels(&[0]).is_err());
        assert!(mi.reorder_levels(&[0, 1, 2]).is_err());
    }

    #[test]
    fn multi_index_reorder_levels_duplicate_index_errors() {
        let mi = MultiIndex::from_tuples(vec![vec!["a".into(), 1_i64.into()]]).unwrap();

        assert!(mi.reorder_levels(&[0, 0]).is_err());
    }

    #[test]
    fn multi_index_reorder_levels_out_of_bounds_errors() {
        let mi = MultiIndex::from_tuples(vec![vec!["a".into(), 1_i64.into()]]).unwrap();

        assert!(mi.reorder_levels(&[0, 5]).is_err());
    }

    // ── is_monotonic / is_lexsorted tests (br-frankenpandas-w4uu) ───────

    #[test]
    fn multi_index_is_monotonic_increasing_on_sorted() {
        // Rows: [(A,1), (A,2), (B,1)] — strictly increasing lexicographic.
        let mi = MultiIndex::from_tuples(vec![
            vec!["A".into(), 1_i64.into()],
            vec!["A".into(), 2_i64.into()],
            vec!["B".into(), 1_i64.into()],
        ])
        .unwrap();
        assert!(mi.is_monotonic_increasing());
        assert!(mi.is_lexsorted());
        assert!(!mi.is_monotonic_decreasing());
    }

    #[test]
    fn multi_index_is_monotonic_decreasing_on_reverse_sorted() {
        // Rows: [(B,2), (B,1), (A,1)].
        let mi = MultiIndex::from_tuples(vec![
            vec!["B".into(), 2_i64.into()],
            vec!["B".into(), 1_i64.into()],
            vec!["A".into(), 1_i64.into()],
        ])
        .unwrap();
        assert!(mi.is_monotonic_decreasing());
        assert!(!mi.is_monotonic_increasing());
    }

    #[test]
    fn multi_index_is_monotonic_both_directions_on_constant_inner() {
        // Equal-level-value rows: [(A,1), (A,1)] — both monotonic trivially.
        let mi = MultiIndex::from_tuples(vec![
            vec!["A".into(), 1_i64.into()],
            vec!["A".into(), 1_i64.into()],
        ])
        .unwrap();
        assert!(mi.is_monotonic_increasing());
        assert!(mi.is_monotonic_decreasing());
    }

    #[test]
    fn multi_index_empty_is_monotonic() {
        let mi = MultiIndex::from_tuples(Vec::new()).unwrap();
        assert!(mi.is_monotonic_increasing());
        assert!(mi.is_monotonic_decreasing());
        assert!(mi.is_lexsorted());
    }

    #[test]
    fn multi_index_single_row_is_monotonic() {
        let mi = MultiIndex::from_tuples(vec![vec!["A".into(), 1_i64.into()]]).unwrap();
        assert!(mi.is_monotonic_increasing());
        assert!(mi.is_monotonic_decreasing());
        assert!(mi.is_lexsorted());
    }

    #[test]
    fn multi_index_unsorted_is_neither() {
        // Rows: [(B,1), (A,2), (B,2)] — unsorted at the outer level.
        let mi = MultiIndex::from_tuples(vec![
            vec!["B".into(), 1_i64.into()],
            vec!["A".into(), 2_i64.into()],
            vec!["B".into(), 2_i64.into()],
        ])
        .unwrap();
        assert!(!mi.is_monotonic_increasing());
        assert!(!mi.is_monotonic_decreasing());
        assert!(!mi.is_lexsorted());
    }

    #[test]
    fn multi_index_outer_ascending_inner_descending_is_not_monotonic() {
        // Rows: [(A,5), (A,1), (B,3)] — outer ascending, inner within A descends.
        let mi = MultiIndex::from_tuples(vec![
            vec!["A".into(), 5_i64.into()],
            vec!["A".into(), 1_i64.into()],
            vec!["B".into(), 3_i64.into()],
        ])
        .unwrap();
        // Lexicographically (A,5) > (A,1) so the "increasing" check fails.
        assert!(!mi.is_monotonic_increasing());
        // (A,5) > (A,1) but (A,1) < (B,3) so decreasing also fails.
        assert!(!mi.is_monotonic_decreasing());
    }

    #[test]
    fn index_lookup_methods_match_pandas() {
        use super::{Index, IndexLabel};
        let i = Index::new(vec![
            IndexLabel::Int64(1),
            IndexLabel::Int64(3),
            IndexLabel::Int64(5),
            IndexLabel::Int64(7),
        ]);

        // get_indexer (exact): -1/None for labels not present (verified vs
        // pandas Index([1,3,5,7]).get_indexer([2,3,6,7]) == [-1,1,-1,3]).
        let target = Index::new(vec![
            IndexLabel::Int64(2),
            IndexLabel::Int64(3),
            IndexLabel::Int64(6),
            IndexLabel::Int64(7),
        ]);
        assert_eq!(
            i.get_indexer(&target),
            vec![None, Some(1), None, Some(3)],
            "get_indexer exact"
        );

        // searchsorted left/right (pandas: 3->1/2, 4->2, 8->4, 0->0).
        assert_eq!(i.searchsorted(&IndexLabel::Int64(3), "left").unwrap(), 1);
        assert_eq!(i.searchsorted(&IndexLabel::Int64(3), "right").unwrap(), 2);
        assert_eq!(i.searchsorted(&IndexLabel::Int64(4), "left").unwrap(), 2);
        assert_eq!(i.searchsorted(&IndexLabel::Int64(8), "left").unwrap(), 4);
        assert_eq!(i.searchsorted(&IndexLabel::Int64(0), "left").unwrap(), 0);

        // asof: last label <= key (pandas: 4->3, 0->NaN, 7->7, 10->7).
        assert_eq!(
            i.asof(&IndexLabel::Int64(4)),
            Some(IndexLabel::Int64(3)),
            "asof 4"
        );
        assert_eq!(i.asof(&IndexLabel::Int64(0)), None, "asof before all");
        assert_eq!(
            i.asof(&IndexLabel::Int64(7)),
            Some(IndexLabel::Int64(7)),
            "asof exact"
        );
        assert_eq!(
            i.asof(&IndexLabel::Int64(10)),
            Some(IndexLabel::Int64(7)),
            "asof after all"
        );

        // factorize: first-appearance order (pandas: ['b','a','b','c'] ->
        // codes [0,1,0,2], uniques ['b','a','c']).
        let f = Index::new(vec![
            IndexLabel::Utf8("b".into()),
            IndexLabel::Utf8("a".into()),
            IndexLabel::Utf8("b".into()),
            IndexLabel::Utf8("c".into()),
        ]);
        let (codes, uniques) = f.factorize();
        assert_eq!(codes, vec![0_isize, 1, 0, 2], "factorize codes");
        assert_eq!(
            uniques.labels(),
            &[
                IndexLabel::Utf8("b".into()),
                IndexLabel::Utf8("a".into()),
                IndexLabel::Utf8("c".into())
            ],
            "factorize uniques"
        );
    }
}
