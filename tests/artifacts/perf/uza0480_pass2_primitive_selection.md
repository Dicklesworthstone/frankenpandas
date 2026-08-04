# br-frankenpandas-uza04.80 Pass 2 Primitive Selection

Timestamp: 2026-06-11T15:52:00-04:00
Worktree: `/data/projects/.scratch/frankenpandas-orangepeak-perf-20260611T1918`
Mode: analysis/artifact only; no source, bead, benchmark, commit, or push changes.

## Live Bead Check

`br show br-frankenpandas-uza04.80 --json` reports:

- Status: `in_progress`
- Assignee: `OrangePeak`
- Parent: `br-frankenpandas-uza04`
- Labels: `csv`, `fp-io`, `no-gaps`, `perf`
- Scope: replace generic CSV Float64 rendering for the all-valid typed writer path only.
- Non-repeat boundary: do not touch `arr72` range indexes, bool-mask witness/verifier work, join checksum lazy scalar materialization, or groupby.

## Fresh Profile Basis

Pass 1 artifacts read:

- `tests/artifacts/perf/uza0480_base_highram_keycard100000_100000x20.json`
- `tests/artifacts/perf/uza0480_base_hyperfine_highram_keycard100000_100000x20.txt`
- `tests/artifacts/perf/uza0480_base_perf_report_children_highram_keycard100000_100000x20.txt`
- `tests/artifacts/perf/uza0480_base_perf_report_nochildren_highram_keycard100000_100000x20.txt`
- `tests/artifacts/perf/uza0480_base_csv_write_read_roundtrip_stable.sha256`
- `tests/artifacts/perf/uza0480_base_csv_write_read_roundtrip_stable_sha256_check.txt`

Head and command:

- HEAD: `98ed64a93d225436dee4ffb15132c711e7570638` (`perf: carry bool selection witness into filter rows`)
- Baseline command: `/data/projects/.scratch/cargo-target-orangepeak-uza0480-base/release-perf/high_ram_perf_baseline --profile uza0480-base --rows 100000 --iters 20 --warmup 3 --frame-cols 10 --key-cardinality 100000`
- Hyperfine command: same binary/profile with `--profile uza0480-base-hyperfine`

Baseline `csv_write_read_roundtrip`:

- mean `90.53049875 ms`
- p50 `88.703384 ms`
- p95 `95.33589 ms`
- p99 `99.875493 ms`
- throughput `1,104,600.122 rows/s`
- throughput `95,121,203.560 bytes/s`
- `rows_out=100000`
- `io_payload_bytes=8611370`
- `checksum=62503875000.0`
- RSS high-water `116440 KiB`

Stable CSV witness:

- SHA: `d93c23c308b1cbba82051d7dd022001e84e5313c0b485a440891a604ebfce367`
- `sha256sum -c tests/artifacts/perf/uza0480_base_csv_write_read_roundtrip_stable.sha256` passed.

Hyperfine full-harness baseline:

- `3.956 s +/- 0.158 s`
- range `3.796 s..4.238 s`
- 10 runs

CPU profile:

- `fp_io::write_pandas_float`: `37.82%` children / `3.19%` self.
- `core::fmt::write`: `34.87%` children / `2.28%` self.
- `core::fmt::float::float_to_decimal_common_shortest::<f64>`: `32.95%` children / `3.90%` self.
- `core::num::imp::flt2dec::strategy::grisu::format_shortest_opt`: `26.61%` children / `14.27%` self.
- No lost samples; profile has 4K cycle samples.

Source basis:

- `try_write_csv_typed` takes the Float64 fast path at `crates/fp-io/src/lib.rs:1732`, stores `FastCol::F(&[f64])` at `:1751-1756`, and calls `write_pandas_float(&mut out, v)` at `:1803-1809`.
- `write_pandas_float` currently uses `write!(out, "{v:?}")` at `crates/fp-io/src/lib.rs:3598-3601`; the common no-scientific case returns after scanning for `e` at `:3602-3605`; the rare scientific path normalizes exponent spelling with `split_off` and push operations at `:3606-3617`.
- Existing focused test coverage asserts pandas-style whole-number `.0`, shortest decimals, signed two-digit scientific notation, infinities, and NaN CSV handling around `crates/fp-io/src/lib.rs:14420-14445`.
- `ryu = "1.0.23"` is already a workspace dependency in `Cargo.toml`; `fp-io` does not yet depend on it directly.
- The installed `ryu` crate documents a pure-Rust fast float-to-decimal algorithm with proof lineage and reports 2-5x faster behavior than std formatting in its own benchmarks (`ryu-1.0.23/src/lib.rs:9-17`, `:64-76`). Its safe `Buffer` API exposes `format` and `format_finite`, with `format_finite` avoiding non-finite checks when the caller already proved finiteness (`ryu-1.0.23/src/buffer/mod.rs:20-79`). The raw `format64` API is re-exported separately and is unsafe (`ryu-1.0.23/src/lib.rs:131-133`, `ryu-1.0.23/src/pretty/mod.rs:13-52`).

## Graveyard and Artifact Mapping

- **Profile-first rule:** `extreme-software-optimization` requires baseline, profile, isomorphism proof, one lever, and score gate. This pass has baseline/profile artifacts and selects only one implementation primitive.
- **Adaptive Compilation & Runtime Specialization, §6.17:** canonical graveyard lines `2131-2142` describe guarded specialized implementations with a generic fallback. Mapping: specialize only the typed Float64 CSV writer's finite-value rendering; keep the current pandas formatter as the fallback/deopt path whenever a guard fails or the wrapper cannot prove byte-equivalence.
- **FrankenSuite Artifact Graph:** summary lines `835-846` require claims to link to exact code, toolchain, datasets, and replayable scenarios. Mapping: Pass 3 must link the formatter claim to HEAD, target-dir/toolchain, the stable CSV witness SHA, high-ram JSON, hyperfine paired/reversed output, and focused fp-io tests.
- **Vectorized Execution + Morsel-Driven Parallelism:** summary lines `951-955` and `979-982` mark cache-sized batch/vectorized processing as a high-impact suite primitive. Mapping: useful as a future CSV writer architecture, but not the one-lever choice here because the measured residual is scalar dtoa under `write_pandas_float`, not row partition scheduling.
- **Hot-loop formatting quick fix:** summary lines `1923-1939` flag hot-loop formatting and recommend writing into reused buffers. Mapping: the current code already appends into the output `String`, so plain buffer reuse is insufficient; the next lever must replace the generic std float formatting primitive itself.
- **Risk countermeasures:** graveyard lines `5808-5812` and summary lines `2268-2270` warn about performance unpredictability and constants. Mapping: use an incumbent, proven dtoa primitive (`ryu`) behind a byte-for-byte pandas compatibility wrapper, then reject if same-worker paired/reversed data does not clear the score gate.

## Candidate Matrix

Score = Impact x Confidence / Effort. Only proceed when Score >= 2.0 and the candidate directly targets the fresh profile.

| Candidate | Impact | Confidence | Effort | Score | Verdict |
| --- | ---: | ---: | ---: | ---: | --- |
| Pandas-compatible `ryu` finite fast path inside `write_pandas_float` / typed Float64 writer | 4 | 3 | 3 | 4.00 | **Select.** It directly replaces the profiled `core::fmt::write` -> `float_to_decimal_common_shortest` -> Grisu chain while preserving the existing typed-writer surface and fallback. Confidence is capped at 3 because `ryu`'s public spelling is not automatically pandas spelling; the wrapper must normalize exponent signs/width and prove the Python boundary cases. |
| Integer-ish Float64 shortcut before generic formatting | 3 | 2 | 4 | 1.50 | Reject. Whole-number `.0` values may exist in this workload, but exact pandas behavior around `-0.0`, finite range, large integral floats, `1e16` boundary, and non-integer decimals makes this a fragile partial shortcut. It also leaves the decimal-heavy Grisu residual mostly intact. |
| Local buffer/split discipline around current std `Debug` formatter | 1 | 4 | 3 | 1.33 | Reject. Replacing `split_off` or reserving scratch space only helps rare scientific notation and small append overhead. The profile says the dominant cost is std float-to-decimal computation, not allocation of the final string cell. |
| Row/column morselization or Rayon-parallel CSV formatting | 3 | 2 | 5 | 1.20 | Reject. It maps to vectorized/morsel execution, but it is too broad for `.80`: row order, quoting, memory pressure, and final concatenation become part of the proof surface while the scalar formatter remains expensive inside each shard. |
| Fork or vendor a custom dtoa formatter into `fp-io` | 4 | 2 | 5 | 1.60 | Reject. It may remove the same hotspot, but it adds provenance, maintenance, and unsafe/algorithmic proof burden when an audited workspace dependency already exposes a safe API. |

## Selected Primitive

Selected primitive: **pandas-compatible `ryu` finite fast path with guarded fallback**.

One-lever implementation shape for Pass 3:

1. Add `ryu = { workspace = true }` to `crates/fp-io/Cargo.toml`.
2. Replace the finite, non-NaN body of `write_pandas_float` with a narrow helper, e.g. `try_write_pandas_float_ryu(out, v) -> bool`.
3. Guard first:
   - NaN remains caller-owned and continues to emit `options.na_rep` in the typed CSV path.
   - Infinities and negative infinity must produce `inf` and `-inf`.
   - Negative zero must remain `-0.0`.
   - Finite values go through one `ryu::Buffer`.
4. Append `ryu::Buffer::format_finite(v)` output to `out` only after pandas normalization:
   - preserve/ensure whole-number `.0`;
   - normalize scientific notation to signed exponent with at least two digits (`1e+16`, `1e-05`);
   - preserve shortest round-trip decimal spelling for ordinary decimals;
   - if a boundary case cannot be proven equivalent to the current pandas-compatible implementation, return `false`.
5. On `false`, call the existing std `Debug`-based body verbatim as the fallback. During Pass 3 this can live as `write_pandas_float_std_fallback` so semantic uncertainty deopts rather than corrupting CSV output.
6. Keep the public `format_pandas_float(v) -> String` and general writer behavior unchanged; the helper is an implementation detail.

This is a single lever: replacing the measured generic formatter primitive for finite Float64 values. It does not alter typed-column detection, row/column order, CSV delimiter/header/index behavior, quoting, scalar fallback conversion, DataFrame layout, join/groupby/filter paths, or beads.

## Isomorphism Proof Obligations

Pass 3 must prove:

- **CSV bytes:** stable witness SHA remains `d93c23c308b1cbba82051d7dd022001e84e5313c0b485a440891a604ebfce367` for the same `csv_write_read_roundtrip` artifact, or any SHA change is an immediate reject unless a pandas oracle explains it and the bead scope is amended.
- **Rows/checksum:** `rows_out=100000`, `io_payload_bytes=8611370`, and `checksum=62503875000.0` remain unchanged for the high-ram profile.
- **Float spelling:** exact current/pandas spelling for the existing unit cases plus added boundary cases: `0.0`, `-0.0`, `1.0`, `100.0`, `0.1`, `1.0/3.0`, `9999999999999999.0`, `1e16`, `1e20`, `1e-4`, `1e-5`, `1e-7`, subnormal finite values, `inf`, `-inf`, and NaN via `na_rep`.
- **Ordering and CSV structure:** row order, column order, header/index inclusion, delimiters, newlines, quote-minimal string cells, and fallback/general writer behavior remain unchanged.
- **Dtype/null behavior:** Float64 typed buffers, all-valid detection, NaN handling, scalar fallback, null/NaN semantics, and dtype promotion surfaces are unchanged.
- **Safety:** safe Rust only in `fp-io`; use `ryu::Buffer` unless there is a separately justified audited wrapper. Do not copy or fork dtoa internals.
- **Performance proof:** same-worker paired and reversed benchmark against Pass 1 baseline; keep only if the observed improvement yields Score >= 2.0 and no p95/p99 regression on the workload.

Fallback/reject triggers:

- Any golden SHA, row count, checksum, or byte-for-byte unit case mismatch.
- Any scientific exponent sign/width mismatch (`1e16` instead of `1e+16`, `1e-5` instead of `1e-05`).
- Any decimal boundary mismatch around Python/Rust/Ryū notation thresholds.
- Any handling change for `NaN`, `inf`, `-inf`, or `-0.0`.
- Need to change `try_write_csv_typed` eligibility, DataFrame layout, parser behavior, or non-float columns.
- Same-worker paired/reversed data fails to clear Score >= 2.0, or p95/p99 regresses materially.

## Recommendation Card

**Primitive:** pandas-compatible `ryu` finite fast path with current std `Debug` formatter as deopt fallback.

**Measured target:** `fp_io::write_pandas_float` at `37.82%` children, dominated by `core::fmt::write` (`34.87%`) and `float_to_decimal_common_shortest` (`32.95%`).

**Forecast score:** Impact 4 x Confidence 3 / Effort 3 = **4.00**. Alien EV `(Impact 4 x Confidence 3 x Reuse 4) / (Effort 3 x AdoptionFriction 1) = 16.0`.

**Contract:** Implement exactly one formatter primitive replacement in `fp-io`; prove byte identity with existing goldens and pandas float spelling tests; re-benchmark same worker; keep only if Score >= 2.0. Otherwise reject and route the residual rather than layering a second formatting shortcut.

**Pass 3 watch points:** `ryu` spelling is not automatically pandas spelling; the dangerous cases are exponent sign/zero-padding, `1e-5` threshold behavior, large integral floats that pandas prints in scientific notation, negative zero, and not accidentally moving NaN handling from `na_rep` into the formatter.
