# fp-frame

DataFrame and Series with pandas-API parity, AACE index alignment,
and GroupBy / Rolling / Resample integration.

Part of the [frankenpandas](https://github.com/Dicklesworthstone/frankenpandas)
workspace. The central data-structure crate; most downstream code
imports from here.

## When to depend on fp-frame directly

Most users should depend on the umbrella `frankenpandas` crate
instead (it re-exports `DataFrame`, `Series`, and the prelude).
Direct dependency makes sense when:

- Building a library that doesn't need the IO layer (`fp-io`) but
  does need DataFrame + Series.
- Embedding DataFrame primitives without the expression parser
  (`fp-expr`) or conformance harness (`fp-conformance`).

## Key types

- `DataFrame` — columnar frame with flat or multi-level row index,
  optional column MultiIndex, per-column validity masks. Supports
  all pandas IO/arithmetic/reshape/groupby surfaces at
  API-parity.
- `Series` — single-column equivalent with pandas-compatible
  string accessor (`.str`), datetime accessor (`.dt`), categorical
  accessor (`.cat`).
- `DataFrameGroupBy` / `SeriesGroupBy` — three automatic execution
  paths (dense Int64 keys, arena-backed, HashMap) chosen per
  group cardinality. 87% speedup on dense paths over the naive
  reference.
- `Rolling` / `Expanding` / `Ewm` / `Resample` — window + resample
  engines, re-used by `DataFrameRolling` / etc.

## Status

Stable surface at API-parity for covered operations (328+ ops per
fp-conformance packet suite). All public error types are
`#[non_exhaustive]` per br-frankenpandas-tne4.

## Links

- [Workspace README](../../README.md) — full API tour.
- [Conformance packets](../fp-conformance/fixtures/) — which
  operations have differential conformance coverage.
- [CHANGELOG](../../CHANGELOG.md)
