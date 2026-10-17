# README Capability Claims Registry (`h5vo9.2`)

This registry maps every headline capability claim in [`README.md`](../README.md) to the in-tree un-`#[ignore]`d Rust test probe and/or CI workflow job that verifies it.

It is enforced by `crates/fp-conformance/src/tests/ci_supply_chain_policy.rs`:
- `readme_capability_claims_registry_maps_to_real_unignored_tests_h5vo9_2` (positive check: verifies each README substring is present in `README.md` and each mapped Rust test exists and is not `#[ignore]`)
- `readme_capability_claims_registry_rejects_missing_or_ignored_probes_h5vo9_2` (negative check: verifies that a missing README claim, a nonexistent test probe, or an `#[ignore]`d test probe fails the gate)

| Claim ID | `README.md` Capability Claim Substring | Probe File | Probe Test (`#[test]`, un-`#[ignore]`d) | CI Workflow Job |
|---|---|---|---|---|
| `CSV_IO` | `read_csv` | `crates/fp-io/src/lib.rs` | `csv_roundtrip_preserves_index_and_values` | `.github/workflows/ci.yml` (`test`) |
| `JSON_IO` | `read_json` | `crates/fp-io/src/lib.rs` | `json_records_orient_roundtrip` | `.github/workflows/ci.yml` (`test`) |
| `PARQUET_IO` | `read_parquet` | `crates/fp-io/src/lib.rs` | `parquet_roundtrip` | `.github/workflows/ci.yml` (`test`) |
| `EXCEL_IO` | `read_excel` | `crates/fp-io/src/lib.rs` | `excel_roundtrip_single_sheet` | `.github/workflows/ci.yml` (`test`) |
| `IPC_FEATHER_IO` | `read_feather` | `crates/fp-io/src/lib.rs` | `ipc_stream_roundtrip` | `.github/workflows/ci.yml` (`test`) |
| `SQL_SQLITE_IO` | `read_sql` | `crates/fp-io/src/lib.rs` | `sql_roundtrip_read_write_table` | `.github/workflows/ci.yml` (`test`) |
| `SQL_POSTGRES_IO` | `PostgresConnection` | `crates/fp-io/src/lib.rs` | `test_postgres_dtype_sql_mappings` | `.github/workflows/ci.yml` (`postgres-integration`) |
| `PICKLE_IO` | `read_pickle` | `crates/fp-io/src/lib.rs` | `pickle_roundtrip_preserves_frame` | `.github/workflows/ci.yml` (`test`) |
| `STATA_IO` | `read_stata` | `crates/fp-io/src/lib.rs` | `stata_roundtrip_preserves_columns_and_values` | `.github/workflows/ci.yml` (`test`) |
| `SAS_IO` | `read_sas` | `crates/fp-io/src/lib.rs` | `read_sas_xport_builds_typed_frame_and_keeps_missing_numeric_as_null_9jvao` | `.github/workflows/ci.yml` (`test`) |
| `EVAL_QUERY` | `DataFrame::eval` | `crates/fp-expr/src/lib.rs` | `eval_adds_two_series` | `.github/workflows/ci.yml` (`test`) |
| `AACE_PLAN` | `align_union` | `crates/fp-index/src/lib.rs` | `align_union_builds_expected_positions_and_nulls` | `.github/workflows/ci.yml` (`test`) |
| `RAPTORQ_SIDECAR` | `RaptorQ` | `crates/fp-conformance/src/lib.rs` | `raptorq_sidecar_roundtrip_and_scrub_succeeds` | `.github/workflows/ci.yml` (`conformance`) |
| `EVIDENCE_LEDGER` | `EvidenceLedger` | `crates/fp-runtime/src/lib.rs` | `strict_policy_rejects_unknown_features` | `.github/workflows/ci.yml` (`test`) |
