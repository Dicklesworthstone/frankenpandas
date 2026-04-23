# fp-join

Merge and join engine (AG-05 leapfrog triejoin) for fp-frame
DataFrames — inner / left / right / outer joins with index
alignment.

Part of the [frankenpandas](https://github.com/Dicklesworthstone/frankenpandas)
workspace.

## AG-05 leapfrog triejoin

The AG-05 algorithm is frankenpandas's leapfrog triejoin
implementation — a generalized multi-way merge that efficiently
computes N-way joins across sorted indices. Used both for direct
`.merge()` calls and for the DataFrame-arithmetic index-alignment
path (`.add`, `.sub`, etc. on frames with non-identical indices).

## When to depend on fp-join directly

Most users should depend on the umbrella `frankenpandas` crate
instead. Direct dependency makes sense when:

- Building a specialized join library that reuses the triejoin
  implementation.
- Prototyping N-way merge semantics.

## Key types

- `MergedDataFrame` — result of an inner/left/right/outer join.
- `JoinType` / `JoinExecutionOptions` — control over arena use,
  arena budget, forced-fallback mode (tested by fuzz_join_series).

## Status

Stable. Fuzz-corpus exercise via fuzz_join_series per br-zjme.

## Links

- [Workspace README](../../README.md)
- [CHANGELOG](../../CHANGELOG.md)
