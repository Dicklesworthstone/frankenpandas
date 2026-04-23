# fp-conformance

Differential conformance harness for frankenpandas against a live
pandas oracle — packet fixtures + fuzz seed tests + live-oracle
parity gates.

Part of the [frankenpandas](https://github.com/Dicklesworthstone/frankenpandas)
workspace.

## What this crate provides

- **Packet fixture runner** — 1,249 JSON fixtures across 430 packet
  IDs under `fixtures/packets/`. Each fixture carries an input, an
  operation, and the expected output computed once by a live pandas
  oracle and captured into the fixture.
- **Live-oracle harness** — 89 `live_oracle_*_matches_pandas` tests
  that dispatch payloads through `oracle/pandas_oracle.py` at
  test time and compare the Rust output against the live pandas
  result. CI enforces live-oracle presence per br-frankenpandas-d6xa.
- **Fuzz entry points** — `fuzz_*_bytes` pub fns (fuzz_csv_parse,
  fuzz_parquet_io, fuzz_semantic_eq, fuzz_format_cross_round_trip,
  fuzz_pivot_table, fuzz_groupby_agg_dispatch, fuzz_rolling_window,
  etc.) consumed by the `fuzz/` crate's libFuzzer targets.
- **Gate topology** — G1..G8 CI gates emit structured JSON forensics
  under `artifacts/ci/gate_forensics.json`.

## How to use

Run the conformance suite locally:

```bash
cargo test -p fp-conformance --lib
```

For live-oracle tests, Python + pandas must be installed. CI
runs `pip install -r oracle/requirements.txt` (pandas version
pinned per br-frankenpandas-boyr).

## When to depend on fp-conformance directly

Rarely. Downstream crates don't depend on the conformance harness
— it's a test apparatus for frankenpandas itself. If you're
building a pandas-parity competitor and want to reuse the fixture
corpus, depending on this crate + the oracle script may be useful.

## Status

Active development. 413/0 tests under `cargo test -p fp-conformance
--lib` at session close (2026-04-23). `HarnessError` is
`#[non_exhaustive]` per br-frankenpandas-tne4.

## Links

- [DISCREPANCIES.md](DISCREPANCIES.md) — documented intentional
  divergences from pandas.
- [COVERAGE.md](COVERAGE.md) — conformance coverage matrix.
- [fixtures/README.md](fixtures/README.md) — fixture format +
  provenance.
- [Workspace README](../../README.md)
