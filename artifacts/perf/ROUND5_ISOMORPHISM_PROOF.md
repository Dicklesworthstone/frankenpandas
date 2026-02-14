# ROUND5 Isomorphism Proof

Change:
- `Index::has_duplicates` now memoizes duplicate detection via `OnceCell<bool>`.

Behavior checks:
- Ordering preserved: yes, index label order unchanged.
- Tie-breaking unchanged: yes, first-occurrence behavior unchanged.
- Floating-point drift: N/A.
- RNG seeds: N/A.

Semantic guards:
- `Index` equality intentionally ignores cache state (labels-only comparison).
- Serde compatibility preserved for existing fixtures (`duplicate_cache` skipped).

Validation artifacts:
- `cargo test -p fp-index` passed (includes cache-state equality regression test).
- `cargo test -p fp-groupby` passed.
- `cargo test -p fp-conformance` passed.
- `cargo run -p fp-conformance --bin fp-conformance-cli -- --write-artifacts --require-green` passed.
- `(cd artifacts/perf && sha256sum -c golden_checksums.txt)` passed.

Performance outcome:
- Mean latency improved from `0.290649768 s` to `0.037208347 s` (`87.20%` faster).
