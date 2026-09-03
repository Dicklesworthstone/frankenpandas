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

/// Helpers that previously owned pandas-visible constructor policy in the
/// harness.  Unlike a raw builder, this shape can make a packet green by
/// rejecting or projecting data before FrankenPandas gets a chance to run.
/// Keep these names forbidden rather than allowing a replacement shadow to
/// return under an innocuous helper name.
const FORBIDDEN_POLICY_HELPERS: [&str; 1] = ["collect_dict_constructor_payloads"];

/// Arms that legitimately construct, each with the reason.
///
/// Repackaging is fine: the fp-join entry points return a result struct
/// (`index` / `columns` / `column_order`) rather than a `DataFrame`, so the arm
/// must reassemble one AFTER delegating. What is NOT fine is computing the
/// answer here.
const ALLOWED: [(&str, &str); 4] = [
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

fn forbidden_policy_helpers(source: &str) -> Vec<String> {
    let source = strip_line_comments(source).join("\n");
    let mut found: Vec<String> = FORBIDDEN_POLICY_HELPERS
        .iter()
        .filter(|name| source.contains(&format!("fn {name}(")))
        .map(|name| (*name).to_owned())
        .collect();
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

#[test]
fn fixture_policy_helpers_do_not_preempt_frankenpandas() {
    let forbidden = forbidden_policy_helpers(&harness_source());
    assert!(
        forbidden.is_empty(),
        "these helpers own pandas-visible policy before FrankenPandas runs: \
         {forbidden:#?}\n\n\
         Delegate the policy to the public fp-* API instead. A helper that \
         rejects, projects, or fabricates constructor payloads can certify the \
         harness rather than the implementation. See br-frankenpandas-oxodo."
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

#[test]
fn policy_helper_is_detected_even_without_a_raw_builder() {
    let source = "\
fn collect_dict_constructor_payloads() {
    return Err(\"column is absent\".to_owned());
}
";
    assert_eq!(
        forbidden_policy_helpers(source),
        vec!["collect_dict_constructor_payloads".to_string()]
    );
}

/// Field names of the struct whose declaration starts with `header`, in
/// source order. Attribute and comment lines are skipped; a field line is
/// `name: Type,` (optionally `pub`).
fn struct_field_names(source: &str, header: &str) -> Vec<String> {
    let start = source
        .find(header)
        .unwrap_or_else(|| panic!("{header} not found in src/lib.rs"));
    let body = &source[start + header.len()..];
    let end = body.find("\n}").expect("struct body must close");
    body[..end]
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            let trimmed = trimmed.strip_prefix("pub ").unwrap_or(trimmed);
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
                return None;
            }
            let (name, _) = trimmed.split_once(':')?;
            let name = name.trim();
            name.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                .then(|| name.to_owned())
        })
        .collect()
}

/// Every option a fixture can carry must reach the oracle.
///
/// br-frankenpandas-00de2. `PacketFixture` (what a test declares and the
/// FrankenPandas arm reads) and `OracleRequest` (what the pandas arm is sent)
/// are two hand-maintained structs. A field added to the first and not the
/// second makes the oracle run pandas' DEFAULT for that option, so the
/// "differential" compares FrankenPandas against a question it was never
/// asked. Nine options had drifted this way (`between_inclusive`, merge_asof
/// `direction` / `tolerance` / `allow_exact_matches` / `by`,
/// `merge_fill_method`, `compare_result_names`, `constructor_copy`,
/// `str_wrap_drop_whitespace`): four live `between` tests pinned the
/// `inclusive="both"` answer, and three merge tests plus a `compare` test
/// failed against pandas the day the oracle first ran. Five more
/// (`str_patterns`, `str_join_from`, `json_*`) made the oracle reject the op,
/// which the harness rendered as a skip. This guard fails on the next one.
#[test]
fn every_fixture_option_reaches_the_oracle_request_00de2() {
    let source = harness_source();
    let fixture_fields = struct_field_names(&source, "pub struct PacketFixture {");
    let request_fields = struct_field_names(&source, "struct OracleRequest {");
    assert!(
        fixture_fields.len() > 150,
        "PacketFixture scan found only {} fields; the scanner is broken",
        fixture_fields.len()
    );
    assert!(
        request_fields.len() > 100,
        "OracleRequest scan found only {} fields; the scanner is broken",
        request_fields.len()
    );

    // Fields that legitimately never travel: identity and provenance
    // metadata, the EXPECTED half of the fixture (the oracle produces it), and
    // the binary round-trip payloads that only the generation path consumes
    // (`capture_live_oracle_response_for_generation` serializes the whole
    // fixture, so they reach the oracle there).
    const NOT_FORWARDED: &[&str] = &[
        "packet_id",
        "case_id",
        "mode",
        "fixture_provenance",
        "oracle_source",
        "requirement_level",
        "retired",
        "expected_series",
        "expected_frame",
        "expected_join",
        "expected_alignment",
        "expected_bool",
        "expected_positions",
        "expected_scalar",
        "expected_dtype",
        "expected_error_contains",
        "excel_input_base64",
        "feather_input_base64",
        "ipc_stream_input_base64",
        "parquet_input_base64",
    ];

    let missing: Vec<&str> = fixture_fields
        .iter()
        .map(String::as_str)
        .filter(|field| !request_fields.iter().any(|forwarded| forwarded == field))
        .filter(|field| !NOT_FORWARDED.contains(field))
        .collect();
    assert!(
        missing.is_empty(),
        "PacketFixture options that never reach OracleRequest — add each to the struct AND to \
         its population in capture_live_oracle_expected, or to NOT_FORWARDED with a reason: \
         {missing:?}"
    );

    // The allowlist must not rot into a rubber stamp either.
    let stale: Vec<&&str> = NOT_FORWARDED
        .iter()
        .filter(|field| !fixture_fields.iter().any(|present| present == *field))
        .collect();
    assert!(
        stale.is_empty(),
        "NOT_FORWARDED names fields PacketFixture no longer has: {stale:?}"
    );
}
