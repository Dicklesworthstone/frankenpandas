#![forbid(unsafe_code)]

use std::cell::OnceCell;
use std::collections::HashMap;
use std::fmt;

use fp_types::Timedelta;
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
/// HashMap fallback for unsorted. Computed lazily via `OnceCell`.
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
    duplicate_cache: OnceCell<bool>,
    /// AG-13: Cached sort order for adaptive backend selection.
    #[serde(skip)]
    sort_order_cache: OnceCell<SortOrder>,
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
            duplicate_cache: OnceCell::new(),
            sort_order_cache: OnceCell::new(),
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
    /// One Vec<IndexLabel> per level, all the same length (= nrows).
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
        let lookup: std::collections::HashSet<&Vec<IndexLabel>> = values
            .iter()
            .filter(|v| v.len() == nlevels)
            .collect();
        if lookup.is_empty() {
            return vec![false; self.len()];
        }
        (0..self.len())
            .map(|row| {
                let key: Vec<IndexLabel> = self
                    .levels
                    .iter()
                    .map(|level| level[row].clone())
                    .collect();
                lookup.contains(&key)
            })
            .collect()
    }

    /// Per-row membership test against values for a single level.
    ///
    /// Matches `pd.MultiIndex.isin(values, level=...)`. Returns `true`
    /// for positions whose label at `level` is in `values`. Returns an
    /// `OutOfBounds` error when `level` exceeds `nlevels()`.
    pub fn isin_level(
        &self,
        values: &[IndexLabel],
        level: usize,
    ) -> Result<Vec<bool>, IndexError> {
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
    use super::{Index, IndexLabel, MultiIndex, align_union, validate_alignment_plan};
    use fp_types::Timedelta;

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
}
