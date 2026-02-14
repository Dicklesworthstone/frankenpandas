#![forbid(unsafe_code)]

use std::cell::OnceCell;
use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum IndexLabel {
    Int64(i64),
    Utf8(String),
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

impl fmt::Display for IndexLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Int64(v) => write!(f, "{v}"),
            Self::Utf8(v) => write!(f, "{v}"),
        }
    }
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
}

/// Detect the sort order of the label slice.
fn detect_sort_order(labels: &[IndexLabel]) -> SortOrder {
    if labels.len() <= 1 {
        return match labels.first() {
            Some(IndexLabel::Int64(_)) | None => SortOrder::AscendingInt64,
            Some(IndexLabel::Utf8(_)) => SortOrder::AscendingUtf8,
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

    SortOrder::Unsorted
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Index {
    labels: Vec<IndexLabel>,
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

    #[must_use]
    pub fn has_duplicates(&self) -> bool {
        *self
            .duplicate_cache
            .get_or_init(|| detect_duplicates(&self.labels))
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
}

pub fn align_union(left: &Index, right: &Index) -> AlignmentPlan {
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

#[cfg(test)]
mod tests {
    use super::{Index, IndexLabel, align_union, validate_alignment_plan};

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
}
