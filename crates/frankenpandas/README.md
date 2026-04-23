# frankenpandas

Clean-room Rust reimplementation of the pandas API — DataFrame,
Series, GroupBy, Rolling, Resample, MultiIndex, 7 IO formats.
Zero unsafe.

**Umbrella crate** re-exporting the public surface of the fp-*
workspace. Most users should depend on `frankenpandas` rather than
individual fp-* crates.

## Install

```toml
[dependencies]
frankenpandas = "0.1"
```

## Quick example

```rust
use frankenpandas::prelude::*;

let df = read_csv_str("name,age,city\nAlice,30,NYC\nBob,25,LA\nCarol,35,NYC")?;
let by_city = df.groupby(&["city"])?.agg_named(&[
    ("mean_age", "age", "mean"),
    ("count", "age", "count"),
])?;
```

## What ships in frankenpandas

- **DataFrame / Series** — pandas-parity API surface (fp-frame).
- **AACE index alignment** — N-way leapfrog triejoin (fp-join,
  fp-index).
- **GroupBy / Rolling / Resample** — with three automatic
  execution paths (fp-groupby).
- **Expression eval / query** — `df.eval` / `df.query` parity
  (fp-expr).
- **7 IO formats** — CSV, JSON, JSONL, Parquet, Excel, Feather,
  Arrow IPC, SQL (fp-io). SQLite today; PostgreSQL / MySQL
  planned under br-frankenpandas-fd90.

## Workspace map

- [`fp-types`](../fp-types/) — core Scalar / DType / NullKind.
- [`fp-columnar`](../fp-columnar/) — columnar storage primitives.
- [`fp-index`](../fp-index/) — Index / MultiIndex.
- [`fp-runtime`](../fp-runtime/) — policies + evidence ledgers.
- [`fp-frame`](../fp-frame/) — DataFrame / Series core.
- [`fp-groupby`](../fp-groupby/) — GroupBy engine.
- [`fp-join`](../fp-join/) — merge / join (AG-05 triejoin).
- [`fp-expr`](../fp-expr/) — expression eval + query parser.
- [`fp-io`](../fp-io/) — IO layer.
- [`fp-conformance`](../fp-conformance/) — differential conformance
  harness against live pandas.
- [`fp-frankentui`](../fp-frankentui/) — interactive TUI surface.

## Status

Pre-release (0.1.0). 3,186 tests green · zero clippy warnings ·
`#![forbid(unsafe_code)]` on every crate · 990 commits since
2026-02-13 · 430 conformance packets across 1,249 fixtures all
green.

See [CHANGELOG.md](../../CHANGELOG.md) for the full history.

## Links

- [Workspace README](../../README.md) — full API tour.
- [Security policy](../../SECURITY.md) — vuln disclosure channel.
- [Authors](../../AUTHORS.md) — swarm identity map.
- [Contribution guide](../../CONTRIBUTING.md) — *(pending)*.
