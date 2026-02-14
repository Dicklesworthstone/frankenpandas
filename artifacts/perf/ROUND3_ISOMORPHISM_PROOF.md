# Round 3 Isomorphism Proof

## Change: Guarded identity-alignment fast path in `fp-groupby::groupby_sum`

- Location: `crates/fp-groupby/src/lib.rs`
- Lever type: skip unnecessary alignment/reindex allocations in a proven identity case

Proof obligations:

- Ordering preserved: yes. Group ordering still follows first-seen key order in scan sequence.
- Tie-breaking unchanged: yes. Existing aggregation/update logic and ordering vector behavior are unchanged.
- Floating-point behavior: identical for accepted rows; arithmetic path for sums is unchanged.
- RNG seeds: unchanged/N/A.
- Golden outputs: `sha256sum -c artifacts/perf/golden_checksums.txt` passed.

Behavior-guard details:

- Fast path is only active when:
  - `keys.index() == values.index()`
  - `!keys.index().has_duplicates()`
- Duplicate-index or non-identical-index cases continue through existing `align_union + reindex` logic.

Regression tests:

- existing deterministic ordering test remains green.
- added duplicate-index behavior test:
  - `groupby_sum_duplicate_equal_index_preserves_alignment_behavior`
  - codifies legacy first-position duplicate alignment semantics.

Conformance evidence:

- `cargo test -p fp-groupby` passed.
- `cargo test -p fp-conformance` passed.
- `cargo run -p fp-conformance --bin fp-conformance-cli -- --write-artifacts --require-green` passed all packets (`FP-P2C-001..FP-P2C-005`).

Performance evidence:

- benchmark delta in `artifacts/perf/ROUND3_BASELINE.md` shows `67.6%` mean improvement on the target workload.

Rollback:

- revert `crates/fp-groupby/src/lib.rs` fast-path block and keep unconditional alignment path.

