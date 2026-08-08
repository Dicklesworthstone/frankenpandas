//! Policy guard: a fixture-operation arm must EXERCISE FrankenPandas, not
//! reimplement it.
//!
//! br-frankenpandas-oxodo. Two defects of this shape have already reached the
//! corpus, and both produced GREEN packets:
//!
//!   * `dataframe_constructor_list_like` kept its own index-length check, so an
//!     fp-frame fix left the fixture red until the copy was fixed identically.
//!   * `dataframe_constructor_scalar` broadcast the scalar itself — and fp-frame
//!     had NO scalar constructor at all, so eight fixtures certified an
//!     operation the library did not expose. A user calling FrankenPandas could
//!     not do what the packet claimed parity for.
//!
//! The second is the dangerous direction: when the harness is MORE capable than
//! the library, the packet is green over a gap, and no amount of fixture
//! regeneration can detect it — the fixture is not the thing lying.
//!
//! So this test pins the set of `execute_*_fixture_operation` arms that build a
//! frame or column with raw constructors. The list is exact in BOTH directions:
//! a new arm that self-builds fails, and an arm that stops self-building also
//! fails until it is removed from the list, which keeps the allowlist from
//! rotting into a rubber stamp.

use std::{fs, path::PathBuf};

/// Raw construction — producing a frame/column directly rather than by calling
/// the operation under test.
///
/// `Scalar::Null(NullKind::` is here because MINTING A MISSING VALUE is the
/// library's job, not the harness's, and an arm that does it is deciding a
/// semantic pandas has a specific answer for. It was added after the original
/// three needles missed
/// `execute_dataframe_constructor_list_like_fixture_operation`: that arm builds
/// its column payloads inline and null-fills them, then hands the result to
/// `collect_dict_constructor_payloads`, so it never touches `Column::from_values`
/// or `DataFrame::new_with_column_order` and the scan walked straight past it —
/// while it was blocking three rows on br-frankenpandas-nywa8.
/// (br-frankenpandas-oxodo)
const RAW_BUILDERS: [&str; 4] = [
    "DataFrame::new_with_column_order",
    "DataFrame::new(",
    "Column::from_values",
    "Scalar::Null(NullKind::",
];

/// Arms that legitimately construct, each with the reason.
///
/// Repackaging is fine: the fp-join entry points return a result struct
/// (`index` / `columns` / `column_order`) rather than a `DataFrame`, so the arm
/// must reassemble one AFTER delegating. What is NOT fine is computing the
/// answer here.
const ALLOWED: [(&str, &str); 5] = [
    (
        "execute_dataframe_merge_fixture_operation",
        "delegates to merge_dataframes_on_with_options; reassembles its parts into a DataFrame",
    ),
    (
        "execute_dataframe_merge_asof_fixture_operation",
        "delegates to fp_join::merge_asof_with_options; reassembles its parts",
    ),
    (
        "execute_dataframe_merge_ordered_fixture_operation",
        "delegates to the ordered-merge entry point; reassembles its parts",
    ),
    (
        "execute_dataframe_fixture_operation",
        "multi-op arm: builds INPUT columns for specific ops (e.g. a mask or a \
         replacement column) before calling the op under test",
    ),
    (
        "execute_dataframe_constructor_list_like_fixture_operation",
        "KNOWN SHADOW, tracked and blocking work — not an approved exception. It \
         builds its column payloads inline and null-fills them instead of \
         reaching DataFrame::from_matrix_rows, which holds three rows on \
         br-frankenpandas-nywa8 behind it. Delegating is NOT mechanical: \
         from_matrix_rows backs both this entry point and from_records, and \
         pandas disagrees between them (DataFrame([[1,2],[3]]) pads and widens \
         col1 to float64; from_records with columns=['a','b','c'] over 2-wide \
         rows RAISES), so it needs a per-entry-point gap policy first. Listed \
         here so the guard stays green and TRUTHFUL rather than being weakened; \
         remove this entry when the arm delegates.",
    ),
];

fn harness_source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");
    fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

/// Line-comment lines are removed before scanning. Without this the guard
/// reports itself: the comments that DOCUMENT a past shadow mention
/// `Column::from_values` by name, and a text scan counts them.
fn strip_line_comments(source: &str) -> Vec<String> {
    source
        .lines()
        .map(|line| {
            if line.trim_start().starts_with("//") {
                String::new()
            } else {
                line.to_string()
            }
        })
        .collect()
}

/// `execute_*_fixture_operation` arms that contain a raw builder, bounded at the
/// next TOP-LEVEL `fn` — not at the next `execute_` fn, which would swallow
/// intervening helpers and blame the wrong function (that mistake initially
/// reported `constructor_kwargs`, whose body delegates cleanly).
fn self_building_arms(source: &str) -> Vec<String> {
    let lines = strip_line_comments(source);
    let is_top_level_fn =
        |line: &str| (line.starts_with("fn ") || line.starts_with("pub fn ")) && line.contains('(');
    let tops: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| is_top_level_fn(line))
        .map(|(idx, _)| idx)
        .collect();

    let mut found = Vec::new();
    for (position, &start) in tops.iter().enumerate() {
        let header = &lines[start];
        let name = header
            .trim_start_matches("pub ")
            .trim_start_matches("fn ")
            .split('(')
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
        if !(name.starts_with("execute_") && name.ends_with("_fixture_operation")) {
            continue;
        }
        let end = tops.get(position + 1).copied().unwrap_or(lines.len());
        let body = lines[start..end].join("\n");
        if RAW_BUILDERS.iter().any(|needle| body.contains(needle)) {
            found.push(name);
        }
    }
    found.sort();
    found
}

#[test]
fn fixture_operation_arms_delegate_to_frankenpandas() {
    let source = harness_source();
    let actual = self_building_arms(&source);
    let mut expected: Vec<String> = ALLOWED.iter().map(|(name, _)| (*name).to_owned()).collect();
    expected.sort();

    let unexpected: Vec<&String> = actual.iter().filter(|a| !expected.contains(a)).collect();
    assert!(
        unexpected.is_empty(),
        "these fixture-operation arms build a frame/column themselves instead of \
         calling the FrankenPandas operation under test:\n  {unexpected:#?}\n\n\
         A packet over a self-built arm certifies the HARNESS, not the library — \
         and if the arm is more capable than fp-frame, it is green over a real \
         gap. Delegate to the real API (adding it to fp-frame if it does not \
         exist yet, as DataFrame::from_scalar had to be), or add the arm to \
         ALLOWED with a reason. See br-frankenpandas-oxodo."
    );

    let stale: Vec<&String> = expected.iter().filter(|e| !actual.contains(e)).collect();
    assert!(
        stale.is_empty(),
        "these arms are on the ALLOWED list but no longer build anything \
         themselves:\n  {stale:#?}\n\nPrune them, so the list keeps meaning \
         something instead of pre-approving whatever appears later."
    );
}

/// The comment-stripping is load-bearing, so it gets its own assertion rather
/// than being trusted implicitly: a doc comment naming a raw builder must not
/// make an arm look like a violator.
#[test]
fn documenting_a_raw_builder_in_a_comment_is_not_a_violation() {
    let source = "\
fn execute_documented_fixture_operation() {
    // This arm used to call Column::from_values and DataFrame::new_with_column_order.
    delegate()
}
";
    assert!(
        self_building_arms(source).is_empty(),
        "a comment mentioning a raw builder must not be read as a call"
    );

    // ...and a real call in the same shape still IS caught.
    let real = "\
fn execute_offender_fixture_operation() {
    let c = Column::from_values(v)?;
}
";
    assert_eq!(
        self_building_arms(real),
        vec!["execute_offender_fixture_operation".to_string()]
    );
}
