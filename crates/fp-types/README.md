# fp-types

Core `Scalar`, `DType`, `NullKind` primitives for the
[frankenpandas](https://github.com/Dicklesworthstone/frankenpandas) workspace.

The universal type system behind `fp-frame` — every DataFrame value,
every column dtype, and every null semantics decision flows through
the types defined here.

## When to depend on fp-types directly

Most users should depend on the umbrella `frankenpandas` crate
instead. Depend on `fp-types` directly only when:

- You are building a frankenpandas-ecosystem crate and need the core
  type vocabulary (`Scalar`, `DType`, `NullKind`) but not the
  DataFrame machinery.
- You are writing a downstream library that conforms to
  frankenpandas's Scalar/DType contract.

## Key types

- `Scalar` — tagged union covering `Int64`, `Float64`, `Utf8`, `Bool`,
  `Null(NullKind)`, `Timedelta64`, `Datetime64`.
- `DType` — type tag for columns (`Int64`, `Float64`, `Utf8`, etc.)
  with promotion rules mirroring pandas.
- `NullKind` — three-variant missing taxonomy (`Null`, `NaN`, `NaT`)
  used for pandas-parity is-missing semantics.
- `Semantic equality` via `Scalar::semantic_eq` — bridges all missing
  kinds (pandas: `NaN == NaN` is `False` in Python; `True` semantically
  when comparing frames).

## Status

Stable surface. Every pub type carries `#[non_exhaustive]` where it
represents an open-ended enumeration (ongoing work under
br-frankenpandas-tne4). Documented contracts under `cargo doc`.

## Links

- [Workspace README](../../README.md)
- [CHANGELOG](../../CHANGELOG.md)
- [Security policy](../../SECURITY.md)
