# br-frankenpandas-3wo1d.1 lower-hex validator state-machine rejection

## Target

- Worktree head before lever: `f8e9bc9ed83d5d1c8cec683a64c84be858e679c1`
- Workload: `join-bench --str-ordered --rows 1000000 --right-rows 1000000 --iters 20`
- Baseline binary: `/data/projects/.scratch/cargo-target-orangepeak-3wo1d1-base/release-perf/join-bench`
- After binary: `/data/projects/.scratch/cargo-target-orangepeak-3wo1d1-after/release-perf/join-bench`

## Baseline

- Build: `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-3wo1d1-base RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-join --profile release-perf --bin join-bench`
- RCH status: failed open locally after worker preflight failures; command remained crate-scoped in a scratch target directory.
- Hyperfine baseline: `102.6 ms +/- 6.0 ms`
- Golden SHA: `2ac49173153820d4b3878817c44be31979faa18b2ae034167f7977adee83b02e`
- Internal line: mean `0.001 ms`, p50 `0.001 ms`, p95 `0.001 ms`, p99 `0.001 ms`, checksum `4999706700.000`

## Profile Evidence

Base profile artifact: `tests/artifacts/perf/perf_report_children_base_3wo1d1_str_ordered_certificate_1000000x1000000.txt`

- `join_bench::build_ordered_utf8_frame`: `72.92%` children.
- `Column::from_f64_values`: `25.74%` self under frame setup.
- `Column::as_lower_hex_sequence_utf8_contiguous` / `OnceLock` certificate first-use: `15.06%` children / `14.95%` self under `merge_once`.

## Lever Attempted

The attempted one-lever source hunk changed `contiguous_utf8_lower_hex_sequence` from parsing every row's fixed-width suffix into `u64` to parsing the first suffix once, carrying an expected lowercase-hex byte state machine, and comparing each later span to those expected bytes. Overflow past the fixed-width suffix and any byte mismatch still returned `None`.

## Isomorphism Proof

- Ordering preserved: yes. The attempted validator changed only certificate discovery, not row position output.
- Tie-breaking unchanged: yes. Matching certificates still fed the same overlap plan, and mismatches still fell back.
- Floating-point unchanged: yes. Payload generation and output scalarization were untouched.
- RNG seeds unchanged/N/A: no RNG in this path.
- Overflow behavior preserved: fixed-width lower-hex counter overflow rejected the certificate instead of wrapping.
- Golden output: after SHA matched baseline exactly: `2ac49173153820d4b3878817c44be31979faa18b2ae034167f7977adee83b02e`; `golden_compare_3wo1d1_str_ordered_lower_hex_state_machine.txt` recorded `golden_cmp=0`.

## Score Gate

Final same-command paired hyperfine:

- Base then after, 30 runs:
  - baseline: `98.0 ms +/- 3.8 ms`
  - after: `104.4 ms +/- 14.4 ms`
  - result: baseline ran `1.06x +/- 0.15` faster
- After then base, 30 runs:
  - after: `98.6 ms +/- 3.0 ms`
  - baseline: `99.6 ms +/- 4.0 ms`
  - result: after ran `1.01x +/- 0.05` faster

Score: below `2.0`; reject. The source hunk was removed before commit.

## Validation

- `cargo fmt -p fp-columnar -p fp-join -- --check`
- `cargo test -p fp-columnar contiguous_utf8_lower_hex_sequence_certificate_jbyuc111111 --lib`
- `cargo check -p fp-join --all-targets`
- `cargo clippy -p fp-join --all-targets -- -D warnings`
- `ubs crates/fp-columnar/src/lib.rs`

UBS exit code was 0 with broad pre-existing `fp-columnar` warning inventory and no critical issues.

## Next Route

Do not iterate this validator family. The refreshed profile points deeper at `Column::from_f64_values` inside ordered UTF8 fixture setup (`25.74%` self) and broader `build_ordered_utf8_frame` setup (`72.92%` children). The next primitive should attack typed value-column construction or production merge-only fixture gating, not lower-hex validation.
