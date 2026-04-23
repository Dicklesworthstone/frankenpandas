# fp-columnar

Columnar storage primitives (`Column`, `ValidityMask`) for pandas-parity
dataframes — the storage layer behind `fp-frame`.

Part of the [frankenpandas](https://github.com/Dicklesworthstone/frankenpandas)
workspace.

## When to depend on fp-columnar directly

Most users should depend on the umbrella `frankenpandas` crate
instead. Depend on `fp-columnar` directly when:

- Embedding frankenpandas-compatible columnar storage in a higher
  crate without the DataFrame layer.
- Implementing a new IO format that serializes columns but routes
  through something other than `fp-io`.

## Key types

- `Column` — typed array of `Scalar` values with associated
  `ValidityMask`. Int64 / Float64 / Utf8 / Bool / Timedelta / Datetime
  specializations. `from_values` infers dtype from input.
- `ValidityMask` — 1-bit-per-element missing-value bitmap. Saves 8x
  memory vs pandas's 8-byte nullable dtype.
- Dtype promotion rules mirror pandas's upcast cascade
  (Int32 -> Int64 -> Float64 -> Object/Utf8 under arithmetic).

## Status

Stable. `#![forbid(unsafe_code)]`. Tested via `fp-frame`'s test suite
and the conformance packets.

## Links

- [Workspace README](../../README.md)
- [CHANGELOG](../../CHANGELOG.md)
