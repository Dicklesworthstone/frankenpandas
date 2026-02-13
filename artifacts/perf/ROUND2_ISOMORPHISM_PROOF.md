# Round 2 Isomorphism Proof

## Change: Borrowed-key alignment maps in `fp-index::align_union`

- Location: `crates/fp-index/src/lib.rs`
- Lever type: allocation-reduction and key-movement minimization

Proof obligations:

- Ordering preserved: yes. Union index emission is still `left labels in order` followed by `first unseen right labels in order`.
- Tie-breaking unchanged: yes. First-position semantics remain identical for duplicate labels (`or_insert(idx)` unchanged).
- Floating-point behavior: N/A. Index alignment path is label-only and does not alter numeric kernels.
- RNG seeds: unchanged/N/A. No randomized behavior in this path.
- Golden outputs: `sha256sum -c artifacts/perf/golden_checksums.txt` passed.

Conformance evidence:

- `cargo test -p fp-index` passed.
- `cargo test -p fp-groupby` passed.
- `cargo run -p fp-conformance --bin fp-conformance-cli -- --write-artifacts --require-green` passed all packets (`FP-P2C-001..FP-P2C-005`).

Performance evidence:

- benchmark delta from `artifacts/perf/ROUND2_BASELINE.md` shows mean runtime improvement of `18.7%` on the alignment-heavy groupby benchmark.

Rollback:

- revert `crates/fp-index/src/lib.rs` to owned-key map construction if any parity drift or pathological hash-behavior regression is detected.

