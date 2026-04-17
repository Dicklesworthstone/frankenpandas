# Adversarial/Fuzz Corpus + Crash Triage Workflow

Adversarial corpus generation, fuzz entrypoints, and crash triage/retention rules for high-risk packet boundaries.
Frozen per bead **bd-2gi.7**.

---

## 1. Adversarial Corpus Architecture

### 1.1 Corpus Layers

The adversarial corpus operates in three layers, each building on the previous:

| Layer | Source | Purpose | Storage |
|---|---|---|---|
| L1: Handcrafted | Manual | Known edge cases from pandas bug trackers, CVEs, and threat matrix (ADV-1..ADV-8) | `crates/fp-conformance/fixtures/packets/` |
| L2: Property-derived | proptest | Shrunk counterexamples that violate invariants | `crates/fp-conformance/fixtures/adversarial/proptest_regressions/` |
| L3: Fuzz-discovered | cargo-fuzz / libFuzzer | Novel crashes and hangs from coverage-guided fuzzing | `crates/fp-conformance/fixtures/adversarial/fuzz_corpus/` |

### 1.2 Corpus Generation Workflow

```
                     ┌─────────────────┐
                     │ Threat Matrix   │ (ADV-1..ADV-8 from bd-2gi.2)
                     │ (manual seeds)  │
                     └────────┬────────┘
                              │
                 ┌────────────▼────────────┐
                 │   L1: Handcrafted       │
                 │   Adversarial Fixtures  │
                 └────────────┬────────────┘
                              │ seed corpus
           ┌──────────────────┼──────────────────┐
           ▼                  ▼                  ▼
  ┌────────────────┐ ┌────────────────┐ ┌────────────────┐
  │ proptest        │ │ cargo-fuzz     │ │ differential   │
  │ (500 cases/run) │ │ (coverage-     │ │ oracle replay  │
  │                 │ │  guided)       │ │                │
  └───────┬────────┘ └───────┬────────┘ └───────┬────────┘
          │ shrink            │ minimize          │ drift
          ▼                   ▼                   ▼
  ┌────────────────┐ ┌────────────────┐ ┌────────────────┐
  │ L2: proptest   │ │ L3: fuzz       │ │ Mismatch       │
  │ regressions    │ │ corpus         │ │ corpus         │
  └───────┬────────┘ └───────┬────────┘ └───────┬────────┘
          │                   │                   │
          └───────────────────┼───────────────────┘
                              ▼
                    ┌─────────────────┐
                    │ Crash Triage    │
                    │ + Retention     │
                    └─────────────────┘
```

### 1.3 Adversarial Input Classes

Cross-reference with `SECURITY_COMPATIBILITY_THREAT_MATRIX.md` (bd-2gi.2):

| Class | Threat Surface | Input Description | Target Crate | Fuzz Priority |
|---|---|---|---|---|
| ADV-1 | TS-1 | Infinite/huge CSV (>1M rows, >1MB fields) | fp-io | P0 |
| ADV-2 | TS-3 | Pathological hash collisions in index labels | fp-index | P0 |
| ADV-3 | TS-6 | All-duplicate join keys (Cartesian explosion) | fp-join | P0 |
| ADV-4 | TS-4, TS-7 | NaN/Inf arithmetic chains | fp-columnar, fp-groupby | P1 |
| ADV-5 | TS-8 | Malformed fixture files (invalid JSON) | fp-conformance | P1 |
| ADV-6 | TS-9 | Tampered RaptorQ sidecars | fp-conformance | P2 |
| ADV-7 | TS-2 | Extreme type coercion (i64::MAX, subnormal, -0.0) | fp-types | P0 |
| ADV-8 | TS-8 | Compromised oracle subprocess | fp-conformance | P2 |

### 1.4 Seed Corpus Construction

For each adversarial class, construct seed inputs:

**ADV-1 Seeds (Data Ingestion):**
```
# adv1_huge_row_count.csv - 100K rows, single column
# adv1_huge_field.csv - 10 rows, one field = 1MB of 'x' characters
# adv1_empty.csv - zero bytes
# adv1_no_newline.csv - single row, no trailing newline
# adv1_only_headers.csv - headers, no data rows
# adv1_10k_columns.csv - 10K empty columns
# adv1_mixed_line_endings.csv - \r\n, \r, \n mixed
```

**ADV-2 Seeds (Hash Collisions):**
```json
{"index_labels": [0, 100, 200, 300, ...]}  // labels chosen to collide in common hash functions
{"index_labels": ["aaa", "aab", "aac", ...]}  // sequential string labels
```

**ADV-3 Seeds (Join Explosion):**
```json
{"left_keys": [1,1,1,...], "right_keys": [1,1,1,...]}  // all-duplicate => N*M output
```

**ADV-7 Seeds (Type Coercion):**
```json
{"values": [9223372036854775807, -9223372036854775808, 1.7976931348623157e308, 5e-324, -0.0, "NaN", "Infinity"]}
```

---

## 2. Fuzz Entrypoints

### 2.1 Entrypoint Registry

Each fuzz target corresponds to a threat surface. Targets are defined as `fuzz_target!` macros consumable by `cargo-fuzz` / libFuzzer.

| Target Name | Crate | Function Under Test | Input Type | ADV Class |
|---|---|---|---|---|
| `fuzz_csv_parse` | fp-io | `read_csv_str()` | `&[u8]` (raw CSV bytes) | ADV-1 |
| `fuzz_scalar_cast` | fp-types | `cast_scalar_owned()` | `(Scalar, DType)` | ADV-7 |
| `fuzz_common_dtype` | fp-types | `common_dtype()` | `(DType, DType)` | ADV-7 |
| `fuzz_index_align` | fp-index | `align_union()` | `(Vec<IndexLabel>, Vec<IndexLabel>)` | ADV-2 |
| `fuzz_series_add` | fp-frame | `Series::add()` | `(Series, Series)` | ADV-4 |
| `fuzz_join_series` | fp-join | `join_series()` | `(Series, Series, JoinType)` | ADV-3 |
| `fuzz_groupby_sum` | fp-groupby | `groupby_sum()` | `(Series, Series, GroupByOptions)` | ADV-4 |
| `fuzz_fixture_parse` | fp-conformance | `serde_json::from_str::<PacketFixture>()` | `&[u8]` (raw JSON bytes) | ADV-5 |
| `fuzz_json_io` | fp-io | `read_json_str()` / `read_jsonl_str()` | `&[u8]` (raw JSON / JSONL text bytes) | ADV-5 |
| `fuzz_excel_io` | fp-io | `read_excel_bytes()` | `&[u8]` (raw workbook bytes) | ADV-1 |
| `fuzz_column_arith` | fp-columnar | `Column::binary_numeric()` | `(Column, Column, BinaryOp)` | ADV-4 |

Implemented entrypoint:

- `fuzz_csv_parse` target: `fuzz/fuzz_targets/fuzz_csv_parse.rs`
- seed corpus: `crates/fp-conformance/fixtures/adversarial/fuzz_corpus/csv_parse/`
- exercises `read_csv_str()` with parse/write/reparse checks via `write_csv_string()`
- `fuzz_fixture_parse` target: `fuzz/fuzz_targets/fuzz_fixture_parse.rs`
- seed corpus: `crates/fp-conformance/fixtures/adversarial/fuzz_corpus/fuzz_fixture_parse/`
- `fuzz_json_io` target: `fuzz/fuzz_targets/fuzz_json_io.rs`
- seed corpus: `crates/fp-conformance/fixtures/adversarial/fuzz_corpus/fuzz_json_io/`
- exercises all `JsonOrient` parse paths plus `read_jsonl_str()` with round-trip checks
- `fuzz_excel_io` target: `fuzz/fuzz_targets/fuzz_excel_io.rs`
- seed corpus: `crates/fp-conformance/fixtures/adversarial/fuzz_corpus/excel_io/`
- exercises `read_excel_bytes()` with a write/reparse oracle via `write_excel_bytes()`
- `fuzz_common_dtype` target: `fuzz/fuzz_targets/fuzz_common_dtype.rs`
- seed corpus: `crates/fp-conformance/fixtures/adversarial/fuzz_corpus/common_dtype/`
- projects raw bytes onto `DType × DType` pairs and checks symmetry plus promotion idempotence for `common_dtype()`

### 2.2 Structured Fuzz Input via `Arbitrary`

For targets that need structured input (not raw bytes), derive `Arbitrary` or implement custom structured fuzzing:

```rust
// Example: structured fuzz input for fuzz_series_add
#[derive(Debug, Arbitrary)]
struct FuzzSeriesAddInput {
    left_labels: Vec<FuzzIndexLabel>,   // max 256 elements
    left_values: Vec<FuzzScalar>,       // same length as labels
    right_labels: Vec<FuzzIndexLabel>,
    right_values: Vec<FuzzScalar>,
}

#[derive(Debug, Arbitrary)]
enum FuzzScalar {
    Int64(i64),
    Float64(f64),           // includes NaN, Inf, -0.0 naturally
    Null,
    NaN,
}

#[derive(Debug, Arbitrary)]
enum FuzzIndexLabel {
    Int64(i64),
    Utf8(String),           // bounded by max_size in fuzz config
}
```

### 2.3 Entrypoint Contracts

Every fuzz target must satisfy these contracts:

1. **No panics**: The function under test must not panic on any input. Expected errors must be returned as `Result::Err`.
2. **No OOM**: Inputs are bounded by configurable limits. Fuzz targets enforce `max_len` on vectors and `max_size` on strings.
3. **No infinite loops**: Fuzz harness runs with a per-invocation timeout of 10 seconds.
4. **Deterministic**: Same input must produce same output (no time-dependent behavior).

### 2.4 Fuzz Configuration

```toml
# fuzz/Cargo.toml
[package]
name = "fp-fuzz"
version = "0.0.0"
publish = false
edition = "2024"

[dependencies]
fp-conformance = { path = "../crates/fp-conformance" }
libfuzzer-sys = "0.4.10"
```

Runtime parameters:
```bash
# Standard fuzz run (10 minutes)
cargo +nightly fuzz run fuzz_csv_parse -- -max_total_time=600 -max_len=65536

# Extended fuzz run (1 hour, for CI nightly)
cargo +nightly fuzz run fuzz_csv_parse -- -max_total_time=3600 -max_len=262144

# With seed corpus
cargo +nightly fuzz run fuzz_csv_parse crates/fp-conformance/fixtures/adversarial/fuzz_corpus/csv_parse/

# Packet fixture parser target
cargo +nightly fuzz run fuzz_fixture_parse \
  crates/fp-conformance/fixtures/adversarial/fuzz_corpus/fuzz_fixture_parse/

# JSON / JSONL ingestion target
cargo +nightly fuzz run fuzz_json_io \
  crates/fp-conformance/fixtures/adversarial/fuzz_corpus/fuzz_json_io/
```

---

## 3. Crash Triage Workflow

### 3.1 Triage Pipeline

```
Crash detected ──> Minimize ──> Classify ──> Deduplicate ──> File ──> Retain
       │                │            │            │           │         │
       ▼                ▼            ▼            ▼           ▼         ▼
  raw crash         cargo-fuzz    severity     stack hash   bead     fixture
  artifact           minimize     rating       comparison   issue    file
```

### 3.2 Severity Classification

| Severity | Criteria | Response Time | Example |
|---|---|---|---|
| **S0: Memory safety** | UB, buffer overrun, use-after-free | Immediate fix | (should not occur: `#![forbid(unsafe_code)]`) |
| **S1: Panic/abort** | Unwinding panic on valid-shaped input | Same session | `unwrap()` on `None` in hot path |
| **S2: Logic divergence** | Output differs from pandas oracle | Next session | Wrong alignment plan for edge case |
| **S3: Resource exhaustion** | OOM or timeout on bounded input | Planned fix | Quadratic join on 1K rows |
| **S4: Cosmetic** | Display/formatting issues | Backlog | Wrong decimal precision in CSV output |

### 3.3 Crash Deduplication

Crashes are deduplicated by **stack hash**: the SHA-256 of the panic backtrace's top 5 frames (function names only, no addresses). Two crashes with the same stack hash are considered duplicates.

```
Stack hash = SHA-256(frame[0].fn_name || frame[1].fn_name || ... || frame[4].fn_name)
```

Deduplication rules:
- Same stack hash within a fuzz target: keep only the smallest minimized input
- Same stack hash across fuzz targets: file as separate entries (different attack surface)
- Same root cause but different stack hash: link as related in bead comments

### 3.4 Crash Minimization

All crashes are minimized before retention:

```bash
# Automatic minimization
cargo +nightly fuzz tmin fuzz_csv_parse <crash_artifact>

# If automatic minimization fails, manual reduction:
# 1. Binary search on input size
# 2. Remove bytes that don't affect crash
# 3. Record minimal reproducer
```

### 3.5 Crash-to-Fixture Promotion

Minimized crashes that reveal real bugs are promoted to permanent regression fixtures:

```
fuzz_corpus/<target>/<crash_hash>.bin
    │
    ▼ (promote)
crates/fp-conformance/fixtures/adversarial/<target>/<crash_hash>.json
    │
    ▼ (if packet-relevant)
crates/fp-conformance/fixtures/packets/fp_p2c_{NNN}_fuzz_{crash_hash}_{mode}.json
```

Promotion criteria:
1. Crash is S0, S1, or S2 severity
2. Fix is implemented and verified
3. Fixture prevents regression

### 3.6 Crash Retention Rules

| Artifact | Retention | Location |
|---|---|---|
| Raw crash input | Until minimized (then delete) | `fuzz/artifacts/<target>/` |
| Minimized crash | Permanent (git-tracked) | `crates/fp-conformance/fixtures/adversarial/fuzz_corpus/<target>/` |
| Crash metadata | Permanent (in bead) | bead comment on the relevant packet bead |
| Promoted fixture | Permanent (git-tracked) | `crates/fp-conformance/fixtures/packets/` |
| Fuzz coverage data | 30 days (ephemeral) | `fuzz/coverage/` (gitignored) |
| Corpus growth artifacts | 7 days (ephemeral) | `fuzz/corpus/<target>/` (gitignored) |

### 3.7 Crash Report Format

Every triaged crash produces a structured report:

```markdown
## Crash Report: {target}/{crash_hash}

- **Target**: fuzz_csv_parse
- **Severity**: S1 (panic)
- **Stack hash**: a1b2c3d4e5f6...
- **Minimized size**: 47 bytes
- **Root cause**: `parse_scalar()` panics on empty string after comma
- **Affected packets**: FP-P2C-001, FP-P2C-008
- **Fix**: Return `Scalar::Null(NullKind::Null)` for empty fields
- **Regression fixture**: `fp_p2c_001_fuzz_a1b2c3d4_strict.json`
- **Bead**: bd-2gi.12.5
```

---

## 4. Integration with Existing Infrastructure

### 4.1 Proptest-to-Fuzz Bridge

Existing proptest strategies (in `proptest_properties.rs`) serve as structured seed generators for fuzz targets:

| Proptest Strategy | Fuzz Target |
|---|---|
| `arb_numeric_series()` | `fuzz_series_add` |
| `arb_index_pair()` | `fuzz_index_align` |
| `arb_groupby_pair()` | `fuzz_groupby_sum` |
| `arb_series_pair()` | `fuzz_join_series` |

Strategy outputs are serialized to the seed corpus directory before fuzz runs.

### 4.2 Conformance Harness Integration

Fuzz-discovered failures that produce valid `PacketFixture` inputs are automatically routed through the differential harness:

```rust
// In fuzz target: if the function doesn't crash but produces output,
// compare against the oracle.
fn check_differential(input: &FuzzInput, output: &Result<Series, FpError>) {
    if let Ok(series) = output {
        // Serialize to fixture format and run through oracle
        let fixture = input.to_packet_fixture();
        let oracle_result = run_oracle_check(&fixture);
        if oracle_result.has_drift() {
            // Write mismatch to adversarial corpus
            write_fuzz_mismatch(&fixture, &oracle_result);
        }
    }
}
```

### 4.3 CI Integration

| Gate | Fuzz Activity | Frequency | Time Budget |
|---|---|---|---|
| G3 (unit) | N/A | Every commit | N/A |
| G4 (property) | proptest 500 cases | Every commit | 60s |
| G6 (conformance) | Regression fixtures only | Every commit | 30s |
| **G4.5 (fuzz)** | Fuzz all P0 targets | Nightly CI | 10 min/target |
| **G4.5-extended** | Fuzz all targets | Weekly CI | 60 min/target |

Gate G4.5 is a new gate inserted between G4 and G5. It runs in nightly CI only (too slow for per-commit).

### 4.4 Relationship to Parity Gates

Fuzz-discovered crashes that reveal parity drift cause the affected packet's parity gate to fail:
- S1/S2 crashes increment `critical_drift_count` in the packet's parity report
- The crash fixture is added to the packet's mismatch corpus
- The packet's gate remains failed until the crash is fixed and the fixture passes

---

## 5. File Layout

```
crates/fp-conformance/
  fixtures/
    adversarial/                          # Adversarial corpus root
      seeds/                              # Handcrafted seeds (L1)
        csv_parse/                        # Seeds for fuzz_csv_parse
        scalar_cast/                      # Seeds for fuzz_scalar_cast
        index_align/                      # Seeds for fuzz_index_align
        ...
      proptest_regressions/               # Shrunk proptest failures (L2)
        <strategy>_<shrunk_hash>.json
      fuzz_corpus/                        # Minimized fuzz crashes (L3, git-tracked)
        csv_parse/
          <crash_hash>.bin
          <crash_hash>.report.md
        scalar_cast/
          ...
    packets/                              # Promoted regression fixtures
      fp_p2c_{NNN}_fuzz_*.json

fuzz/                                     # cargo-fuzz workspace (future)
  Cargo.toml
  fuzz_targets/
    fuzz_csv_parse.rs
    fuzz_scalar_cast.rs
    fuzz_common_dtype.rs
    fuzz_index_align.rs
    fuzz_series_add.rs
    fuzz_join_series.rs
    fuzz_groupby_sum.rs
    fuzz_fixture_parse.rs
    fuzz_column_arith.rs
  artifacts/                              # Raw crash artifacts (gitignored)
  corpus/                                 # Fuzz corpus growth (gitignored)
  coverage/                               # Coverage data (gitignored)
```

---

## 6. Operational Checklist

### 6.1 Adding a New Fuzz Target

1. Identify the function under test and its threat surface class
2. Add entry to the Entrypoint Registry (Section 2.1)
3. Define structured input type with `#[derive(Arbitrary)]` or use raw bytes
4. Create seed corpus from handcrafted adversarial inputs
5. Verify target compiles: `cargo +nightly fuzz build <target>`
6. Run initial 10-minute fuzz session
7. Triage any initial crashes
8. Add to CI gate G4.5 configuration

### 6.2 Responding to a Crash

1. **Minimize**: `cargo +nightly fuzz tmin <target> <crash_artifact>`
2. **Classify**: Assign severity S0-S4
3. **Deduplicate**: Compute stack hash, check against known crashes
4. **File**: Add crash report as bead comment on the relevant packet bead
5. **Fix**: Implement fix, verify crash no longer reproduces
6. **Promote**: Create regression fixture from minimized crash
7. **Verify**: Run full conformance suite to confirm no regressions

### 6.3 Corpus Maintenance

- **Weekly**: Merge fuzz corpus across CI runs (corpus distillation)
- **Monthly**: Review retained crashes, archive resolved S3/S4 entries
- **Per-release**: Verify all promoted fixtures still pass

---

## Changelog

- **bd-2gi.7** (2026-02-14): Initial adversarial/fuzz corpus design. Defines three-layer corpus architecture (handcrafted, proptest-derived, fuzz-discovered), 9 fuzz entrypoints mapped to 8 adversarial input classes, four-severity crash triage pipeline with stack-hash deduplication, crash-to-fixture promotion workflow, CI integration gates (G4.5/G4.5-extended), and operational checklists.
