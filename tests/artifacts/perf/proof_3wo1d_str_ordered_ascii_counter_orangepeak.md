# br-frankenpandas-3wo1d ordered UTF8 ASCII-counter setup proof

## Target

- Worktree head before change: `d8d31a098c07bc7ae8a2c7aa378b49cfcadc9264`
- Workload: `join-bench --str-ordered --rows 1000000 --right-rows 1000000 --iters 20`
- Baseline binary: `/data/projects/.scratch/cargo-target-orangepeak-3wo1d-base/release-perf/join-bench`
- After binary: `/data/projects/.scratch/cargo-target-orangepeak-3wo1d-after/release-perf/join-bench`

## Baseline

- Build: `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-3wo1d-base RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-join --profile release-perf --bin join-bench`
- RCH status: failed open locally after worker preflight failures; command remained crate-scoped in a scratch target directory.
- Hyperfine baseline: `159.6 ms +/- 5.6 ms`
- Golden SHA: `2ac49173153820d4b3878817c44be31979faa18b2ae034167f7977adee83b02e`
- Internal line: mean `0.001 ms`, p50 `0.001 ms`, p95 `0.002 ms`, p99 `0.002 ms`, checksum `4999706700.000`

## Profile Evidence

Base profile artifact: `tests/artifacts/perf/perf_report_children_base_3wo1d_str_ordered_1000000x1000000.txt`

- `join_bench::build_ordered_utf8_frame`: `70.80%` children / `25.24%` self
- Lower-hex certificate first-use under `merge_once`: `22.05%` children
- DataFrame drop: `3.98%`
- Numeric checksum: `2.98%`

## Change

`build_ordered_utf8_frame` now uses a deterministic carried ASCII counter for ordered lower-hex fixture keys when `key_start..key_start+rows` fits in eight lowercase hex digits. The existing `push_id_lower_hex_8` formatter remains the fallback for oversized ranges or checked-add overflow.

## Isomorphism Proof

- Ordering preserved: yes. The loop still appends rows in ascending `key_start + row` order and pushes offsets after each key.
- Tie-breaking unchanged: yes. Merge inputs and duplicate/cardinality behavior are unchanged; only fixture key byte construction changed.
- Floating-point: identical. Payload value generation is untouched.
- RNG seeds: unchanged/N/A. This benchmark path uses no RNG.
- Fallback behavior: ranges beyond `u32::MAX` or overflow use the existing formatter.
- Golden output: after SHA matched baseline exactly: `2ac49173153820d4b3878817c44be31979faa18b2ae034167f7977adee83b02e`; `golden_compare_3wo1d_str_ordered_ascii_counter.txt` recorded `golden_cmp=0`.

## Score Gate

Final same-command paired hyperfine:

- Base then after, 30 runs:
  - baseline: `110.8 ms +/- 8.3 ms`
  - after: `100.7 ms +/- 5.1 ms`
  - result: after ran `1.10x +/- 0.10` faster
- After then base, 30 runs:
  - after: `102.1 ms +/- 3.3 ms`
  - baseline: `105.6 ms +/- 5.3 ms`
  - result: after ran `1.03x +/- 0.06` faster

Conservative score: Impact 2 x Confidence 3 / Effort 2 = `3.0`; keep.

## After Profile

After profile artifact: `tests/artifacts/perf/perf_report_children_after_3wo1d_str_ordered_ascii_counter_1000000x1000000.txt`

- `join_bench::build_ordered_utf8_frame`: `65.52%` children / `16.93%` self
- Lower-hex certificate first-use is now the leading self bucket in the short profile.
- Next route: certificate first-use/prewarming or a production merge-only gate, not another key-format loop.

## Validation

- `cargo fmt -p fp-join -- --check`
- `cargo check -p fp-join --all-targets`
- `cargo clippy -p fp-join --all-targets -- -D warnings`
- `ubs crates/fp-join/src/bin/join-bench.rs`

UBS exit code was 0. The remaining warnings are pre-existing benchmark-file heuristics outside the new helper: `expect` in `golden_dump`, quantile indexing, and `format!` in golden output construction.
