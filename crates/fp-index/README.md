# fp-index

Flat and multi-level row-index types (`Index`, `MultiIndex`) for
pandas-parity dataframes.

Part of the [frankenpandas](https://github.com/Dicklesworthstone/frankenpandas)
workspace.

## When to depend on fp-index directly

Most users should depend on the umbrella `frankenpandas` crate
instead. Direct dependency makes sense when:

- Building a library that needs pandas-style row-axis identity /
  alignment without the full DataFrame machinery.
- Implementing a custom DataFrame that reuses frankenpandas's index
  primitives.

## Key types

- `Index` — flat row index over `IndexLabel` values
  (`Int64`, `Utf8`, `Timedelta64`, `Datetime64`).
- `MultiIndex` — hierarchical row index with level arrays +
  optional level names. Supports `from_tuples`, `from_arrays`,
  `from_product`, `get_level_values`, `droplevel`, `swaplevel`,
  `reorder_levels`, `to_flat_index`.
- `MultiIndexOrIndex` — shape-polymorphic accessor returning the
  logical row axis as either variant.
- Alignment algebra via `AACE` (AG-01..05 leapfrog triejoin) used by
  `fp-join` and `fp-frame` arithmetic.

## Status

Foundation stable. Row MultiIndex integration into `fp-frame`'s
DataFrame row axis landed via br-frankenpandas-1zzp (closed
2026-04-22).

## Links

- [Workspace README](../../README.md)
- [CHANGELOG](../../CHANGELOG.md)
