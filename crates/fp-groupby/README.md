# fp-groupby

GroupBy engine for `fp-frame` with dense-key / arena-backed /
hash-map execution paths.

Part of the [frankenpandas](https://github.com/Dicklesworthstone/frankenpandas)
workspace.

## Execution paths (automatic)

GroupBy operations pick one of three execution strategies at
runtime based on key cardinality and dtype:

1. **Dense-Int64** — when group keys are Int64 and range ≤ 65,536,
   uses O(1) array indexing. Fastest path.
2. **Arena-backed** — medium cardinality uses
   [bumpalo](https://crates.io/crates/bumpalo)'s arena allocator.
   Single malloc, zero per-group fragmentation.
3. **HashMap fallback** — unbounded cardinality uses a HashMap
   with source-index references (avoids per-group `Vec` clones).

All three paths produce identical results, verified by property-based
tests and the conformance suite.

## When to depend on fp-groupby directly

Most users should depend on the umbrella `frankenpandas` crate
instead. Direct dependency makes sense when:

- Building a specialized aggregation library that reuses the
  three-path execution dispatcher.
- Prototyping new GroupBy semantics outside the DataFrame context.

## Status

Stable. Row MultiIndex emit on multi-key groupby landed via
br-frankenpandas-1zzp.2 (closed 2026-04-22) — previously our
groupby flattened multi-keys to `"a|1"` strings.

## Links

- [Workspace README](../../README.md)
- [CHANGELOG](../../CHANGELOG.md)
