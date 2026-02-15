# Performance Baselines + Extreme Optimization Protocol

Baseline benchmarks, profiling workflow, and behavior-isomorphism evidence requirements.
Frozen per bead **bd-2gi.8**.

---

## 1. Baseline Benchmark Suite

### 1.1 Core Operations

Every optimization must be measured against these baselines:

| Operation | Benchmark | Input Scale | Metric |
|---|---|---|---|
| `Series::add` (aligned) | series_add_aligned | 10K rows, same index | ops/s |
| `Series::add` (union) | series_add_union | 10K rows, 50% overlap | ops/s |
| `join_series` (inner) | join_inner | 10K x 10K rows | ops/s |
| `join_series` (left) | join_left | 10K x 10K rows | ops/s |
| `groupby_sum` | groupby_sum | 10K rows, 100 groups | ops/s |
| `read_csv_str` | csv_parse | 100K rows x 10 cols | MB/s |
| `write_csv_string` | csv_write | 100K rows x 10 cols | MB/s |
| `Index::align_union` | index_align | 10K x 10K labels | ops/s |
| `Column::from_values` | column_build | 10K scalars | ops/s |
| `Column::binary_numeric` | column_arith | 10K values, Add | ops/s |

### 1.2 Measurement Protocol

```bash
# Step 1: Capture baseline (before optimization)
hyperfine --warmup 3 --min-runs 10 'cargo test -p <crate> --release -- <bench_test>'

# Step 2: Capture flamegraph (debug profile for symbols)
cargo flamegraph --test <test_name> -p <crate>

# Step 3: Capture syscall profile
strace -c -f cargo test -p <crate> --release -- <bench_test> 2> strace_before.txt
```

### 1.3 Artifact Naming Convention

```
artifacts/perf/
  ROUND{N}_BASELINE.md              # Before/after metrics
  ROUND{N}_OPPORTUNITY_MATRIX.md    # Identified hotspots
  ROUND{N}_RECOMMENDATION_CONTRACT.md  # Proposed change + EV
  ROUND{N}_ISOMORPHISM_PROOF.md     # Behavioral equivalence evidence
  round{N}_{op}_hyperfine_before.json
  round{N}_{op}_hyperfine_after.json
  round{N}_{op}_flamegraph_before.svg
  round{N}_{op}_flamegraph_after.svg
  round{N}_{op}_strace_before.txt
  round{N}_{op}_strace_after.txt
```

---

## 2. Extreme Optimization Protocol

### 2.1 Five-Phase Workflow

Every optimization follows this sequence:

```
Phase 1: PROFILE → Phase 2: IDENTIFY → Phase 3: CONTRACT → Phase 4: IMPLEMENT → Phase 5: PROVE
```

#### Phase 1: Profile

Measure current performance with hyperfine, flamegraph, and strace. Record in `ROUND{N}_BASELINE.md`.

#### Phase 2: Identify

Analyze flamegraph to find hotspots. List top-3 opportunities ranked by estimated impact in `ROUND{N}_OPPORTUNITY_MATRIX.md`.

#### Phase 3: Contract

Write a recommendation contract specifying:
- **Change description**: What will be modified
- **Hotspot evidence**: Which profile data justifies this change
- **Expected impact**: Quantitative prediction (e.g., "30% throughput improvement")
- **Risk assessment**: What could go wrong
- **Rollback plan**: How to revert if the change regresses
- **Isomorphism proof plan**: How behavioral equivalence will be demonstrated

Record in `ROUND{N}_RECOMMENDATION_CONTRACT.md`.

#### Phase 4: Implement

Apply the change. Must pass all gates (G1..G8 from COVERAGE_FLAKE_BUDGETS.md).

#### Phase 5: Prove

Measure performance after the change. Document behavioral equivalence evidence in `ROUND{N}_ISOMORPHISM_PROOF.md`.

---

## 3. Behavior-Isomorphism Evidence Requirements

### 3.1 Mandatory Evidence

Every optimization must provide ALL of the following:

| Evidence | Description | Tool |
|---|---|---|
| **Test parity** | All existing tests pass unchanged | `cargo test --workspace` |
| **Golden output** | CSV/JSON output unchanged for golden inputs | `sha256sum` comparison |
| **Conformance green** | All packet fixtures pass | `cargo test -p fp-conformance` |
| **Differential oracle** | No new drift in differential harness | `run_differential_suite()` |
| **Property hold** | All property tests pass at original case count | proptest |

### 3.2 Sufficient Evidence (at least one)

| Evidence | Description |
|---|---|
| **Algebraic proof** | Mathematical argument that output is identical |
| **Exhaustive trace** | Log comparison showing identical intermediate states |
| **Fuzzer pass** | No new failures after 10 minutes of fuzzing |
| **Golden checksum match** | SHA-256 of output matches pre-change golden checksum |

### 3.3 Golden Checksum Registry

Golden checksums are stored in `artifacts/perf/golden_checksums.txt`. Format:

```
<sha256>  <operation>  <input_description>
```

After each optimization round, checksums must be verified:

```bash
sha256sum -c artifacts/perf/golden_checksums.txt
```

---

## 4. Performance Regression Detection

### 4.1 Regression Threshold

An optimization that causes a regression in ANY benchmark must be rejected unless:
1. The regression is < 5% AND the improvement in the target benchmark is > 20%
2. The regression is documented and approved in a bead comment

### 4.2 Noise Floor

Performance measurements have a noise floor of ~5%. Changes within the noise floor are not considered regressions or improvements.

### 4.3 CI Performance Gate

The CI performance gate (future) will:
1. Run the benchmark suite on release profile
2. Compare against stored baselines
3. Fail if any benchmark regresses > 10%
4. Warn if any benchmark regresses > 5%

---

## 5. Profiling Tools

### Required Tools

| Tool | Purpose | Install |
|---|---|---|
| `hyperfine` | Benchmark timing | `cargo install hyperfine` |
| `flamegraph` | CPU profile visualization | `cargo install flamegraph` |
| `strace` | Syscall profiling | system package |
| `perf` | Linux perf counters | system package |

### Optional Tools

| Tool | Purpose |
|---|---|
| `cargo-llvm-cov` | Coverage measurement |
| `heaptrack` | Memory allocation profiling |
| `valgrind --tool=cachegrind` | Cache miss analysis |

---

## 6. Completed Optimization Rounds

| Round | Target | Change | Result |
|---|---|---|---|
| 1 | Packet conformance | Baseline establishment | N/A (baseline) |
| 2 | GroupBy sum | Index hash optimization | Throughput improvement |
| 3 | GroupBy sum | Key deduplication | Memory reduction |
| 4 | GroupBy sum | Generic fallback path | Code simplification |
| 5 | GroupBy sum | Final polish | Minor improvements |
| 6 | ASUPERSYNC runtime policy (`decide_join_admission`) | Borrowed evidence labels + static join evidence/loss constants | Name-allocation elimination (`11008 -> 0` bytes / 256 calls) and median latency improvement (`430ns -> 380ns`) |
| 7 | FRANKENTUI replay bundle assembly (`build_frankentui_e2e_replay_bundles`) | Indexed fallback mode lookup (`HashMap<(&str,&str), RuntimeMode>`) replacing repeated linear scans | Stress-path latency reduction (`p50: 20.6ms -> 7.5ms`) and fallback lookup-step collapse (`33,718,464 -> 65,664`) |

Rounds 1-5 are documented in `artifacts/perf/ROUND{1..5}_*.md` with full hyperfine, flamegraph, strace, and isomorphism proof artifacts.

Round 6 (`bd-2gi.27.8`) evidence snapshot:

```bash
rch exec -- cargo test -p fp-runtime --lib \
  asupersync_join_admission_profile_snapshot_reports_allocation_delta -- --nocapture
```

Observed output (2026-02-15):
- Baseline `p50/p95/p99` (ns): `430 / 571 / 5029`
- Optimized `p50/p95/p99` (ns): `380 / 2695 / 4458`
- Name allocation bytes (256 calls): `11008 -> 0`

Isomorphism proof command:

```bash
rch exec -- cargo test -p fp-runtime --lib \
  asupersync_join_admission_optimized_path_is_isomorphic_to_baseline
```

This captures the required single-lever optimization loop for ASUPERSYNC-H:
baseline, one optimization lever, parity proof, and re-baseline.

Round 7 (`bd-2gi.28.8`) evidence snapshot:

```bash
rch exec -- cargo test -p fp-frankentui --lib \
  e2e_replay_bundle_profile_snapshot_reports_lookup_delta -- --nocapture
```

Observed output (2026-02-15):
- Baseline `p50/p95/p99` (ns): `20614447 / 24878999 / 26117420`
- Optimized `p50/p95/p99` (ns): `7511826 / 8672051 / 9427574`
- Fallback mode-lookup steps (64 iterations, amplified workload):
  `33718464 -> 65664`

Isomorphism proof command:

```bash
rch exec -- cargo test -p fp-frankentui --lib \
  e2e_replay_bundle_optimized_path_is_isomorphic_to_baseline
```

This captures the required single-lever optimization loop for FRANKENTUI-H:
baseline, one optimization lever, parity proof, and re-baseline.

---

## 7. Optimization Priority Matrix

When choosing what to optimize next, rank by:

```
Priority = (Estimated Impact × Confidence) / (Effort × Risk)
```

| Factor | Scale | Description |
|---|---|---|
| Estimated Impact | 1-5 | Expected throughput/memory improvement |
| Confidence | 1-5 | How certain is the impact estimate |
| Effort | 1-5 | Implementation complexity |
| Risk | 1-5 | Likelihood of introducing bugs |

This matches the EV scoring used in the Alien Graveyard (bd-2t5e) bead specifications.

---

## Changelog

- **bd-2gi.8** (2026-02-14): Initial performance baselines document. Defines core benchmark suite, five-phase optimization protocol, behavior-isomorphism evidence requirements, golden checksum registry, regression detection policy, and optimization priority matrix. References 5 completed optimization rounds.
- **bd-2gi.27.8** (2026-02-15): Added ASUPERSYNC runtime-policy optimization round snapshot (borrowed evidence labels + static join evidence/loss constants), with explicit profile output and baseline-vs-optimized isomorphism test commands.
- **bd-2gi.28.8** (2026-02-15): Added FRANKENTUI replay-bundle optimization snapshot (indexed fallback mode lookup), including stress-path p50/p95/p99 deltas and lookup-step delta with explicit isomorphism proof command.
