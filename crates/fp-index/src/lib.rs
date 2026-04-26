#![forbid(unsafe_code)]

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

use std::{collections::HashMap, fmt, sync::OnceLock};

use fp_types::{Scalar, Timedelta};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum IndexLabel {
    Int64(i64),
    Utf8(String),
    Timedelta64(i64),
    Datetime64(i64),
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
            Self::Int64(_) | Self::Utf8(_) => false,
        }
    }
}

fn index_label_is_truthy(label: &IndexLabel) -> bool {
    if label.is_missing() {
        return false;
    }
    match label {
        IndexLabel::Int64(v) => *v != 0,
        IndexLabel::Utf8(s) => !s.is_empty(),
        IndexLabel::Timedelta64(v) => *v != 0,
        IndexLabel::Datetime64(v) => *v != 0,
    }
}

impl fmt::Display for IndexLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Int64(v) => write!(f, "{v}"),
            Self::Utf8(v) => write!(f, "{v}"),
            Self::Timedelta64(v) => write!(f, "{}", Timedelta::format(*v)),
            Self::Datetime64(v) => write!(f, "{}", format_datetime_ns(*v)),
        }
    }
}

pub fn format_datetime_ns(nanos: i64) -> String {
    if nanos == i64::MIN {
        return "NaT".to_owned();
    }
    let secs = nanos / 1_000_000_000;
    let subsec_nanos = (nanos % 1_000_000_000).unsigned_abs() as u32;
    let dt = chrono::DateTime::from_timestamp(secs, subsec_nanos)
        .unwrap_or(chrono::DateTime::UNIX_EPOCH);
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
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

/// Detect the sort order of the label slice.
fn detect_sort_order(labels: &[IndexLabel]) -> SortOrder {
    if labels.len() <= 1 {
        return match labels.first() {
            Some(IndexLabel::Int64(_)) | None => SortOrder::AscendingInt64,
            Some(IndexLabel::Utf8(_)) => SortOrder::AscendingUtf8,
            Some(IndexLabel::Timedelta64(_)) => SortOrder::AscendingTimedelta64,
            Some(IndexLabel::Datetime64(_)) => SortOrder::AscendingDatetime64,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Index {
    labels: Vec<IndexLabel>,
    /// Optional name for the index (matches pandas `Index.name`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip)]
    duplicate_cache: OnceLock<bool>,
    /// AG-13: Cached sort order for adaptive backend selection.
    #[serde(skip)]
    sort_order_cache: OnceLock<SortOrder>,
}

impl PartialEq for Index {
    fn eq(&self, other: &Self) -> bool {
        self.labels == other.labels
    }
}

impl Eq for Index {}

fn detect_duplicates(labels: &[IndexLabel]) -> bool {
    let mut seen = HashMap::<&IndexLabel, ()>::new();
    for label in labels {
        if seen.insert(label, ()).is_some() {
            return true;
        }
    }
    false
}

impl Index {
    #[must_use]
    pub fn new(labels: Vec<IndexLabel>) -> Self {
        Self {
            labels,
            name: None,
            duplicate_cache: OnceLock::new(),
            sort_order_cache: OnceLock::new(),
        }
    }

    #[must_use]
    pub fn from_i64(values: Vec<i64>) -> Self {
        Self::new(values.into_iter().map(IndexLabel::from).collect())
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
        &self.labels
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
        *self
            .duplicate_cache
            .get_or_init(|| detect_duplicates(&self.labels))
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
        *self
            .sort_order_cache
            .get_or_init(|| detect_sort_order(&self.labels))
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

    fn position_map_first_ref(&self) -> HashMap<&IndexLabel, usize> {
        let mut positions = HashMap::with_capacity(self.labels.len());
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
        let map = self.position_map_first_ref();
        target
            .labels
            .iter()
            .map(|label| map.get(label).copied())
            .collect()
    }

    #[must_use]
    pub fn isin(&self, values: &[IndexLabel]) -> Vec<bool> {
        let set: HashMap<&IndexLabel, ()> = values.iter().map(|v| (v, ())).collect();
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
        let mut seen = HashMap::<&IndexLabel, ()>::new();
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
        match keep {
            DuplicateKeep::First => {
                let mut seen = HashMap::<&IndexLabel, ()>::new();
                for (i, label) in self.labels.iter().enumerate() {
                    if seen.insert(label, ()).is_some() {
                        result[i] = true;
                    }
                }
            }
            DuplicateKeep::Last => {
                let mut seen = HashMap::<&IndexLabel, ()>::new();
                for (i, label) in self.labels.iter().enumerate().rev() {
                    if seen.insert(label, ()).is_some() {
                        result[i] = true;
                    }
                }
            }
            DuplicateKeep::None => {
                let mut counts = HashMap::<&IndexLabel, usize>::new();
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
        let other_set = other.position_map_first_ref();
        let mut seen = HashMap::<&IndexLabel, ()>::new();
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

    #[must_use]
    pub fn union_with(&self, other: &Self) -> Self {
        let mut seen = HashMap::<&IndexLabel, ()>::new();
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
        let other_set = other.position_map_first_ref();
        let mut seen = HashMap::<&IndexLabel, ()>::new();
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
        let mut seen = HashMap::<&IndexLabel, ()>::new();
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
        let start = start.min(self.labels.len());
        let end = start.saturating_add(len).min(self.labels.len());
        self.propagate_name(Self::new(self.labels[start..end].to_vec()))
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
                    IndexLabel::Utf8(s) => s
                        .parse::<i64>()
                        .map_or_else(|_| l.clone(), IndexLabel::Int64),
                    IndexLabel::Timedelta64(ns) => IndexLabel::Int64(*ns),
                    IndexLabel::Datetime64(ns) => IndexLabel::Int64(*ns),
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
                    IndexLabel::Utf8(_) => l.clone(),
                    IndexLabel::Timedelta64(ns) => IndexLabel::Utf8(Timedelta::format(*ns)),
                    IndexLabel::Datetime64(ns) => IndexLabel::Utf8(format_datetime_ns(*ns)),
                })
                .collect(),
        ))
    }

    /// Equality check against another Index.
    ///
    /// Matches `pd.Index.equals(other)`. Returns true iff `other` has
    /// the same labels in the same order. Names are ignored (use
    /// `identical` for a name-sensitive check).
    #[must_use]
    pub fn equals(&self, other: &Self) -> bool {
        self.labels == other.labels
    }

    /// Strict equality including name.
    ///
    /// Matches `pd.Index.identical(other)`. Requires the same labels in
    /// the same order AND the same name.
    #[must_use]
    pub fn identical(&self, other: &Self) -> bool {
        self.labels == other.labels && self.name == other.name
    }

    fn value_counts_raw(
        &self,
        sort: bool,
        ascending: bool,
        dropna: bool,
    ) -> (Vec<(IndexLabel, usize)>, usize) {
        let mut seen_order: Vec<IndexLabel> = Vec::new();
        let mut counts: HashMap<IndexLabel, usize> = HashMap::new();
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
                IndexLabel::Int64(_) | IndexLabel::Timedelta64(_) | IndexLabel::Datetime64(_) => 8,
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
        self.labels.clone()
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
        let mut labels = self.labels.clone();
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
        let mut labels = self.labels.clone();
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
        let mut labels = self.labels.clone();
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

fn index_position_groups(index: &Index) -> HashMap<IndexLabel, Vec<usize>> {
    let mut groups: HashMap<IndexLabel, Vec<usize>> = HashMap::new();
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

    AlignmentPlan {
        union_index: Index::new(output_labels),
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

    AlignmentPlan {
        union_index: Index::new(union_labels),
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
    let mut seen: std::collections::HashSet<IndexLabel> = std::collections::HashSet::with_capacity(
        indexes.iter().map(|idx| idx.labels().len()).sum(),
    );
    let mut union_labels: Vec<IndexLabel> = Vec::new();
    for idx in indexes {
        for label in idx.labels() {
            if seen.insert(label.clone()) {
                union_labels.push(label.clone());
            }
        }
    }
    let union = Index::new(union_labels);

    // Build position maps for each input
    let maps: Vec<HashMap<&IndexLabel, usize>> = indexes
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
    #[error("freq must be positive")]
    NonPositiveFreq,
    #[error("cannot compute range: end < start with positive freq")]
    InvalidRange,
}

/// Create a TimedeltaIndex with evenly spaced values.
///
/// Analogous to `pd.timedelta_range()`. Must specify at least two of:
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
        (Some(s), Some(_e), Some(p)) => {
            if p == 0 {
                (s, 0)
            } else if p == 1 {
                (s, 1)
            } else {
                (s, p)
            }
        }
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
        return Ok(dt.and_utc().timestamp_nanos_opt().unwrap_or(i64::MIN));
    }
    if let Ok(dt) = NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S") {
        return Ok(dt.and_utc().timestamp_nanos_opt().unwrap_or(i64::MIN));
    }

    // Try date-only format (midnight)
    if let Ok(date) = chrono::NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        let dt = date.and_hms_opt(0, 0, 0).unwrap();
        return Ok(dt.and_utc().timestamp_nanos_opt().unwrap_or(i64::MIN));
    }

    Err(DateRangeError::ParseError(trimmed.to_owned()))
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
/// Analogous to `pd.date_range()`. Must specify at least two of:
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
            let n = ((e - s) / freq + 1) as usize;
            (s, n)
        }
        (Some(s), None, Some(p)) => (s, p),
        (None, Some(e), Some(p)) => {
            let s = e - (p.saturating_sub(1) as i64) * freq;
            (s, p)
        }
        (Some(s), Some(_e), Some(p)) => {
            if p == 0 {
                (s, 0)
            } else if p == 1 {
                (s, 1)
            } else {
                (s, p)
            }
        }
        _ => return Err(DateRangeError::InsufficientParams),
    };

    let nanos: Vec<i64> = (0..count).map(|i| start_val + (i as i64) * freq).collect();
    let mut idx = Index::from_datetime64(nanos);
    if let Some(n) = name {
        idx = idx.set_name(n);
    }
    Ok(idx)
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

    /// Set the names for all levels.
    #[must_use]
    pub fn set_names(mut self, names: Vec<Option<String>>) -> Self {
        // Pad or truncate to match nlevels.
        self.names = names;
        self.names.resize(self.nlevels(), None);
        self
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

    /// Compute a non-unique indexer against another MultiIndex.
    ///
    /// Matches `pd.MultiIndex.get_indexer_non_unique(target)` by expanding
    /// every matching source position for each target tuple in target order.
    /// Missing target tuples contribute a single `-1` entry and their target
    /// position is recorded in the returned `missing` vector.
    #[must_use]
    pub fn get_indexer_non_unique(&self, target: &Self) -> (Vec<isize>, Vec<usize>) {
        if self.nlevels() != target.nlevels() {
            return (vec![-1; target.len()], (0..target.len()).collect());
        }

        let mut positions = HashMap::<Vec<IndexLabel>, Vec<usize>>::with_capacity(self.len());
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
        let mut first_seen: HashMap<Vec<IndexLabel>, usize> = HashMap::with_capacity(len);
        let mut counts: HashMap<Vec<IndexLabel>, usize> = HashMap::with_capacity(len);

        for row in 0..len {
            let key: Vec<IndexLabel> = self.levels.iter().map(|level| level[row].clone()).collect();
            *counts.entry(key.clone()).or_insert(0) += 1;
            first_seen.entry(key).or_insert(row);
        }

        match keep {
            DuplicateKeep::First => {
                for row in 0..len {
                    let key: Vec<IndexLabel> =
                        self.levels.iter().map(|level| level[row].clone()).collect();
                    if first_seen[&key] != row {
                        out[row] = true;
                    }
                }
            }
            DuplicateKeep::Last => {
                let mut last_seen: HashMap<Vec<IndexLabel>, usize> = HashMap::with_capacity(len);
                for row in 0..len {
                    let key: Vec<IndexLabel> =
                        self.levels.iter().map(|level| level[row].clone()).collect();
                    last_seen.insert(key, row);
                }
                for row in 0..len {
                    let key: Vec<IndexLabel> =
                        self.levels.iter().map(|level| level[row].clone()).collect();
                    if last_seen[&key] != row {
                        out[row] = true;
                    }
                }
            }
            DuplicateKeep::None => {
                for row in 0..len {
                    let key: Vec<IndexLabel> =
                        self.levels.iter().map(|level| level[row].clone()).collect();
                    if counts[&key] > 1 {
                        out[row] = true;
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
        let lookup: std::collections::HashSet<&Vec<IndexLabel>> =
            values.iter().filter(|v| v.len() == nlevels).collect();
        if lookup.is_empty() {
            return vec![false; self.len()];
        }
        (0..self.len())
            .map(|row| {
                let key: Vec<IndexLabel> =
                    self.levels.iter().map(|level| level[row].clone()).collect();
                lookup.contains(&key)
            })
            .collect()
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
        let lookup: std::collections::HashSet<&IndexLabel> = values.iter().collect();
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
    use fp_types::{Scalar, Timedelta};

    use super::{
        DateOffset, Index, IndexLabel, MultiIndex, align_union, apply_date_offset, bdate_range,
        infer_freq_from_timestamps, validate_alignment_plan,
    };

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
    fn index_equality_ignores_duplicate_cache_state() {
        let index_with_cache = Index::new(vec!["a".into(), "a".into(), "b".into()]);
        assert!(index_with_cache.has_duplicates());

        let fresh_index = Index::new(vec!["a".into(), "a".into(), "b".into()]);
        assert_eq!(index_with_cache, fresh_index);
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
}
