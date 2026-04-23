# fp-runtime

Runtime policies, evidence ledgers, and
[asupersync](https://crates.io/crates/asupersync) interop for the
frankenpandas execution layer.

Part of the [frankenpandas](https://github.com/Dicklesworthstone/frankenpandas)
workspace.

## When to depend on fp-runtime directly

Most users should depend on the umbrella `frankenpandas` crate
instead. Direct dependency makes sense when:

- Building a frankenpandas-compatible execution harness that needs
  policy control over alignment / coercion / error handling.
- Interoperating with the asupersync concurrency system (enable the
  `asupersync` Cargo feature).

## Key types

- `RuntimePolicy` — strict / hardened / lenient mode selection for
  DataFrame / Series operations. Controls when dtype promotion errors
  vs silently coerces.
- `EvidenceLedger` — structured audit trail for every alignment
  decision. Useful for debugging / replay.

## Features

- `asupersync` — enables `asupersync::Outcome<T, E>` interop for
  cancellable / panicking operations. Requires
  [asupersync](https://crates.io/crates/asupersync) as a runtime
  dep.

## Status

Stable. Pinned to asupersync `0.3.1` (verified in UPGRADE_LOG.md).

## Links

- [Workspace README](../../README.md)
- [CHANGELOG](../../CHANGELOG.md)
