//! Guard: a fixture frame payload keeps the COLUMN ORDER its JSON was written
//! in, so FrankenPandas and the oracle are handed the same frame.
//!
//! br-frankenpandas-i9mgp. `FixtureDataFrame.columns` was a `BTreeMap`, which
//! sorts the names. The oracle reads the same JSON into a Python dict, which
//! preserves document order. So for a frame written `{lk1, lk2, left_v}` the two
//! sides received DIFFERENT frames — FP got `left_v, lk1, lk2` — and the
//! difference surfaced far away, as a column-order mismatch in the merge output
//! of FP-P2D-033. FrankenPandas was not at fault: its merge faithfully preserved
//! the order it was given.
//!
//! That is the dangerous shape — the harness mis-parses the INPUT, so no amount
//! of re-banking the EXPECTED side can fix it, and the failure points at the
//! wrong crate.

use super::{FixtureDataFrame, resolve_frame_column_order};

fn frame_from_json(json: &str) -> FixtureDataFrame {
    serde_json::from_str(json).expect("fixture frame parses")
}

/// The names below are deliberately chosen so document order and sorted order
/// DISAGREE: sorted is `left_v, lk1, lk2`, document is `lk1, lk2, left_v`. A
/// fixture whose names happen to already be sorted cannot detect this bug, which
/// is why the corpus carried it for so long.
const UNSORTED_DOCUMENT_ORDER: &str = r#"{
    "index": [{"kind": "int64", "value": 0}],
    "columns": {
        "lk1": [{"kind": "int64", "value": 1}],
        "lk2": [{"kind": "utf8", "value": "a"}],
        "left_v": [{"kind": "int64", "value": 10}]
    }
}"#;

#[test]
fn frame_payload_keeps_its_json_column_order_i9mgp() {
    let frame = frame_from_json(UNSORTED_DOCUMENT_ORDER);
    assert_eq!(
        frame.columns.document_order(),
        ["lk1".to_owned(), "lk2".to_owned(), "left_v".to_owned()],
        "document order, not the map's sorted keys"
    );
    assert_eq!(
        resolve_frame_column_order(&frame).expect("order resolves"),
        vec!["lk1".to_owned(), "lk2".to_owned(), "left_v".to_owned()]
    );
    // The map access every other harness site relies on still works.
    assert!(frame.columns.contains_key("left_v"));
    assert_eq!(frame.columns.len(), 3);
}

#[test]
fn an_explicit_column_order_still_wins_i9mgp() {
    // Without this, "just use document order" would silently override the
    // fixtures that state their axis deliberately.
    let frame = frame_from_json(
        r#"{
        "index": [{"kind": "int64", "value": 0}],
        "column_order": ["left_v", "lk1", "lk2"],
        "columns": {
            "lk1": [{"kind": "int64", "value": 1}],
            "lk2": [{"kind": "utf8", "value": "a"}],
            "left_v": [{"kind": "int64", "value": 10}]
        }
    }"#,
    );
    assert_eq!(
        resolve_frame_column_order(&frame).expect("order resolves"),
        vec!["left_v".to_owned(), "lk1".to_owned(), "lk2".to_owned()],
        "an explicit column_order outranks the document order"
    );
}

#[test]
fn a_partial_explicit_order_appends_the_rest_in_document_order_i9mgp() {
    // The half that is easy to get wrong: the named columns lead, and the
    // REMAINDER must still follow the document rather than the sorted keys.
    let frame = frame_from_json(
        r#"{
        "index": [{"kind": "int64", "value": 0}],
        "column_order": ["left_v"],
        "columns": {
            "lk1": [{"kind": "int64", "value": 1}],
            "lk2": [{"kind": "utf8", "value": "a"}],
            "left_v": [{"kind": "int64", "value": 10}]
        }
    }"#,
    );
    assert_eq!(
        resolve_frame_column_order(&frame).expect("order resolves"),
        vec!["left_v".to_owned(), "lk1".to_owned(), "lk2".to_owned()]
    );
}

#[test]
fn column_order_still_rejects_a_missing_or_duplicate_name_i9mgp() {
    // The validation this chokepoint already performed must survive the change;
    // an ordering fix must not become a way to smuggle a bad axis through.
    let missing = frame_from_json(
        r#"{
        "index": [{"kind": "int64", "value": 0}],
        "column_order": ["nope"],
        "columns": {"lk1": [{"kind": "int64", "value": 1}]}
    }"#,
    );
    assert!(resolve_frame_column_order(&missing).is_err());

    let duplicate = frame_from_json(
        r#"{
        "index": [{"kind": "int64", "value": 0}],
        "column_order": ["lk1", "lk1"],
        "columns": {"lk1": [{"kind": "int64", "value": 1}]}
    }"#,
    );
    assert!(resolve_frame_column_order(&duplicate).is_err());
}

#[test]
fn round_trip_writes_document_order_back_i9mgp() {
    // Re-banking a fixture must not silently re-sort its columns, or every
    // regeneration would rewrite axes it was not asked to touch.
    let frame = frame_from_json(UNSORTED_DOCUMENT_ORDER);
    let written = serde_json::to_string(&frame.columns).expect("serializes");
    let lk1 = written.find("lk1").expect("lk1 present");
    let lk2 = written.find("lk2").expect("lk2 present");
    let left_v = written.find("left_v").expect("left_v present");
    assert!(
        lk1 < lk2 && lk2 < left_v,
        "serialized column order must follow the document, got {written}"
    );
}
